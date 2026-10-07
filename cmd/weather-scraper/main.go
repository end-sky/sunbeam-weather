// SPDX-License-Identifier: GPL-3.0-or-later
// Sunbeam Weather backend. Network access stays here so the Rust GUI does not
// need an HTTP client, API keys, or a background daemon.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	appVersion = "SunbeamWeather/0.1"
	timeout    = 10 * time.Second
	cacheTTL   = 5 * time.Minute
)

type Location struct {
	ID        int64   `json:"id"`
	Name      string  `json:"name"`
	Admin1    *string `json:"admin1,omitempty"`
	Country   *string `json:"country,omitempty"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
	Timezone  *string `json:"timezone,omitempty"`
}

type Current struct {
	TempC        float64  `json:"temp_c"`
	FeelsC       float64  `json:"feels_c"`
	Humidity     *float64 `json:"humidity,omitempty"`
	WindKph      *float64 `json:"wind_kph,omitempty"`
	WindDir      *string  `json:"wind_dir,omitempty"`
	PressureHpa  *float64 `json:"pressure_hpa,omitempty"`
	VisibilityKm *float64 `json:"visibility_km,omitempty"`
	UV           *float64 `json:"uv,omitempty"`
	PrecipMm     *float64 `json:"precip_mm,omitempty"`
	CloudPct     *float64 `json:"cloud_pct,omitempty"`
	Condition    string   `json:"condition"`
	Icon         string   `json:"icon"`
}

type Hourly struct {
	Time       string   `json:"time"`
	TempC      float64  `json:"temp_c"`
	PrecipProb *float64 `json:"precip_prob,omitempty"`
	PrecipMm   *float64 `json:"precip_mm,omitempty"`
	WindKph    *float64 `json:"wind_kph,omitempty"`
	Icon       string   `json:"icon"`
}

type Daily struct {
	Date       string   `json:"date"`
	Condition  string   `json:"condition"`
	Icon       string   `json:"icon"`
	MaxC       float64  `json:"max_c"`
	MinC       float64  `json:"min_c"`
	PrecipProb *float64 `json:"precip_prob,omitempty"`
	Sunrise    *string  `json:"sunrise,omitempty"`
	Sunset     *string  `json:"sunset,omitempty"`
}

type SourceStatus struct {
	Name string `json:"name"`
	OK   bool   `json:"ok"`
	MS   uint64 `json:"ms"`
	Note string `json:"note"`
}

type Consensus struct {
	SourceCount        int      `json:"source_count"`
	TemperatureSpreadC *float64 `json:"temperature_spread_c,omitempty"`
	Message            string   `json:"message"`
}

type Report struct {
	Location  Location       `json:"location"`
	FetchedAt string         `json:"fetched_at"`
	Current   Current        `json:"current"`
	Hourly    []Hourly       `json:"hourly"`
	Daily     []Daily        `json:"daily"`
	Sources   []SourceStatus `json:"sources"`
	Consensus Consensus      `json:"consensus"`
}

type sourceResult struct {
	name    string
	current Current
	hourly  []Hourly
	daily   []Daily
	ms      uint64
	err     error
}

func main() {
	if len(os.Args) < 2 {
		die("usage: weather-scraper search <query> | weather <lat> <lon> <label> <c|f> <contact>")
	}
	switch os.Args[1] {
	case "search":
		if len(os.Args) < 3 {
			die("missing search query")
		}
		runSearch(strings.Join(os.Args[2:], " "))
	case "weather":
		if len(os.Args) < 7 {
			die("usage: weather-scraper weather <lat> <lon> <label> <c|f> <contact>")
		}
		lat, err := strconv.ParseFloat(os.Args[2], 64)
		if err != nil {
			die("invalid latitude")
		}
		lon, err := strconv.ParseFloat(os.Args[3], 64)
		if err != nil {
			die("invalid longitude")
		}
		contact := os.Args[6]
		runWeather(Location{Name: os.Args[4], Latitude: lat, Longitude: lon}, contact)
	default:
		die("unknown command: " + os.Args[1])
	}
}

func runSearch(query string) {
	result, err := geocode(query)
	if err != nil {
		die(err.Error())
	}
	writeJSON(result)
}

func runWeather(loc Location, contact string) {
	if cached, ok := readCache(loc); ok {
		writeJSON(cached)
		return
	}
	sources := make(chan sourceResult, 3)
	var wg sync.WaitGroup
	wg.Add(3)
	go func() {
		defer wg.Done()
		sources <- timedSource("Open-Meteo", func(ctx context.Context) (Current, []Hourly, []Daily, error) { return fetchOpenMeteo(ctx, loc) })
	}()
	go func() {
		defer wg.Done()
		sources <- timedSource("wttr.in", func(ctx context.Context) (Current, []Hourly, []Daily, error) { return fetchWttr(ctx, loc) })
	}()
	go func() {
		defer wg.Done()
		sources <- timedSource("MET Norway", func(ctx context.Context) (Current, []Hourly, []Daily, error) { return fetchMET(ctx, loc, contact) })
	}()
	wg.Wait()
	close(sources)

	var results []sourceResult
	for r := range sources {
		results = append(results, r)
	}
	report, err := mergeReport(loc, results)
	if err != nil {
		die(err.Error())
	}
	writeCache(loc, report)
	writeJSON(report)
}

func timedSource(name string, fn func(context.Context) (Current, []Hourly, []Daily, error)) sourceResult {
	start := time.Now()
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cur, hourly, daily, err := fn(ctx)
	return sourceResult{name: name, current: cur, hourly: hourly, daily: daily, ms: uint64(time.Since(start).Milliseconds()), err: err}
}

func geocode(query string) ([]Location, error) {
	endpoint := "https://geocoding-api.open-meteo.com/v1/search?name=" + url.QueryEscape(query) + "&count=8&language=en&format=json"
	var payload struct {
		Results []Location `json:"results"`
	}
	if err := getJSON(context.Background(), endpoint, "Accept", "application/json", &payload); err != nil {
		return nil, err
	}
	return payload.Results, nil
}

func fetchOpenMeteo(ctx context.Context, loc Location) (Current, []Hourly, []Daily, error) {
	endpoint := fmt.Sprintf("https://api.open-meteo.com/v1/forecast?latitude=%f&longitude=%f&current=temperature_2m,apparent_temperature,weather_code,relative_humidity_2m,wind_speed_10m,wind_direction_10m,surface_pressure,visibility,uv_index,precipitation,cloud_cover&hourly=temperature_2m,precipitation_probability,precipitation,wind_speed_10m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset&timezone=auto&forecast_days=7", loc.Latitude, loc.Longitude)
	var p openMeteoResponse
	if err := getJSON(ctx, endpoint, "Accept", "application/json", &p); err != nil {
		return Current{}, nil, nil, err
	}
	cur := Current{
		TempC:    p.Current.Temperature,
		FeelsC:   p.Current.Apparent,
		Humidity: ptr(p.Current.Humidity), WindKph: ptr(p.Current.WindSpeed),
		WindDir:     ptr(fmt.Sprintf("%s°", fmtFloat(p.Current.WindDirection))),
		PressureHpa: ptr(p.Current.SurfacePressure), VisibilityKm: ptr(p.Current.Visibility / 1000),
		UV: ptr(p.Current.UV), PrecipMm: ptr(p.Current.Precip), CloudPct: ptr(p.Current.Cloud),
	}
	cur.Condition, cur.Icon = weatherCode(p.Current.WeatherCode, true)
	hourly := make([]Hourly, 0, 24)
	n := minInt(18, len(p.Hourly.Time))
	for i := 0; i < n; i++ {
		icon, _ := weatherCode(p.Hourly.WeatherCode[i], isDayHour(p.Hourly.Time[i]))
		hourly = append(hourly, Hourly{Time: p.Hourly.Time[i], TempC: p.Hourly.Temperature[i], PrecipProb: ptr(p.Hourly.PrecipProb[i]), PrecipMm: ptr(p.Hourly.Precip[i]), WindKph: ptr(p.Hourly.WindSpeed[i]), Icon: icon})
	}
	daily := make([]Daily, 0, len(p.Daily.Time))
	for i := range p.Daily.Time {
		c, ic := weatherCode(p.Daily.WeatherCode[i], true)
		daily = append(daily, Daily{Date: p.Daily.Time[i], Condition: c, Icon: ic, MaxC: p.Daily.Max[i], MinC: p.Daily.Min[i], PrecipProb: ptr(p.Daily.PrecipProb[i]), Sunrise: ptr(p.Daily.Sunrise[i]), Sunset: ptr(p.Daily.Sunset[i])})
	}
	return cur, hourly, daily, nil
}

type openMeteoResponse struct {
	Current struct {
		Time            float64 `json:"time"`
		Temperature     float64 `json:"temperature_2m"`
		Apparent        float64 `json:"apparent_temperature"`
		WeatherCode     int     `json:"weather_code"`
		Humidity        float64 `json:"relative_humidity_2m"`
		WindSpeed       float64 `json:"wind_speed_10m"`
		WindDirection   float64 `json:"wind_direction_10m"`
		SurfacePressure float64 `json:"surface_pressure"`
		Visibility      float64 `json:"visibility"`
		UV              float64 `json:"uv_index"`
		Precip          float64 `json:"precipitation"`
		Cloud           float64 `json:"cloud_cover"`
	} `json:"current"`
	Hourly struct {
		Time        []string  `json:"time"`
		Temperature []float64 `json:"temperature_2m"`
		PrecipProb  []float64 `json:"precipitation_probability"`
		Precip      []float64 `json:"precipitation"`
		WindSpeed   []float64 `json:"wind_speed_10m"`
		WeatherCode []int     `json:"weather_code"`
	} `json:"hourly"`
	Daily struct {
		Time        []string  `json:"time"`
		Max         []float64 `json:"temperature_2m_max"`
		Min         []float64 `json:"temperature_2m_min"`
		PrecipProb  []float64 `json:"precipitation_probability_max"`
		WeatherCode []int     `json:"weather_code"`
		Sunrise     []string  `json:"sunrise"`
		Sunset      []string  `json:"sunset"`
	} `json:"daily"`
}

func fetchWttr(ctx context.Context, loc Location) (Current, []Hourly, []Daily, error) {
	q := url.QueryEscape(fmt.Sprintf("%.5f,%.5f", loc.Latitude, loc.Longitude))
	endpoint := "https://wttr.in/" + q + "?format=j1"
	var root wttrEnvelope
	if err := getJSON(ctx, endpoint, "User-Agent", appVersion+" (Linux desktop weather app)", &root); err != nil {
		return Current{}, nil, nil, err
	}
	cur := root.Current()
	if cur == nil {
		return Current{}, nil, nil, errors.New("wttr.in returned no current conditions")
	}
	current := normalizeWttrCurrent(*cur)
	daily := normalizeWttrDaily(root.Weather())
	hourly := normalizeWttrHourly(root.Weather())
	return current, hourly, daily, nil
}

type wttrEnvelope struct {
	CurrentCondition []wttrCurrent `json:"current_condition"`
	WeatherData      []wttrDay     `json:"weather"`
	Data             *struct {
		CurrentCondition []wttrCurrent `json:"current_condition"`
		WeatherData      []wttrDay     `json:"weather"`
	} `json:"data,omitempty"`
}

func (w wttrEnvelope) Current() *wttrCurrent {
	if len(w.CurrentCondition) > 0 {
		return &w.CurrentCondition[0]
	}
	if w.Data != nil && len(w.Data.CurrentCondition) > 0 {
		return &w.Data.CurrentCondition[0]
	}
	return nil
}
func (w wttrEnvelope) Weather() []wttrDay {
	if len(w.WeatherData) > 0 {
		return w.WeatherData
	}
	if w.Data != nil {
		return w.Data.WeatherData
	}
	return nil
}

type wttrCurrent struct {
	TempC       string `json:"temp_C"`
	FeelsC      string `json:"FeelsLikeC"`
	Humidity    string `json:"humidity"`
	WindSpeed   string `json:"windspeedKmph"`
	WindDir     string `json:"winddir16Point"`
	Pressure    string `json:"pressure"`
	Visibility  string `json:"visibility"`
	UV          string `json:"uvIndex"`
	Precip      string `json:"precipMM"`
	Cloud       string `json:"cloudcover"`
	WeatherCode string `json:"weatherCode"`
	WeatherDesc []struct {
		Value string `json:"value"`
	} `json:"weatherDesc"`
}
type wttrDay struct {
	Date    string `json:"date"`
	Max     string `json:"maxtempC"`
	Min     string `json:"mintempC"`
	Sunrise []struct {
		Time string `json:"sunrise"`
	} `json:"sunrise"`
	Sunset []struct {
		Time string `json:"sunset"`
	} `json:"sunset"`
	Hourly []wttrHour `json:"hourly"`
}
type wttrHour struct {
	Time   string `json:"time"`
	Temp   string `json:"tempC"`
	Chance string `json:"chanceofrain"`
	Precip string `json:"precipMM"`
	Wind   string `json:"windspeedKmph"`
	Code   string `json:"weatherCode"`
	Desc   []struct {
		Value string `json:"value"`
	} `json:"weatherDesc"`
}

func normalizeWttrCurrent(x wttrCurrent) Current {
	desc := firstDesc(x.WeatherDesc)
	cur := Current{TempC: parseFloat(x.TempC), FeelsC: parseFloat(x.FeelsC), Humidity: parsePtr(x.Humidity), WindKph: parsePtr(x.WindSpeed), WindDir: ptr(x.WindDir), PressureHpa: parsePtr(x.Pressure), VisibilityKm: parsePtr(x.Visibility), UV: parsePtr(x.UV), PrecipMm: parsePtr(x.Precip), CloudPct: parsePtr(x.Cloud), Condition: desc}
	cur.Icon = iconFromText(desc)
	return cur
}
func normalizeWttrDaily(days []wttrDay) []Daily {
	out := make([]Daily, 0, minInt(7, len(days)))
	for _, d := range days {
		desc := dayDesc(d)
		pp := maxRainProb(d.Hourly)
		var sr, ss *string
		if len(d.Sunrise) > 0 {
			sr = ptr(d.Sunrise[0].Time)
		}
		if len(d.Sunset) > 0 {
			ss = ptr(d.Sunset[0].Time)
		}
		out = append(out, Daily{Date: d.Date, Condition: desc, Icon: iconFromText(desc), MaxC: parseFloat(d.Max), MinC: parseFloat(d.Min), PrecipProb: pp, Sunrise: sr, Sunset: ss})
		if len(out) == 7 {
			break
		}
	}
	return out
}
func normalizeWttrHourly(days []wttrDay) []Hourly {
	out := []Hourly{}
	for _, d := range days {
		for _, h := range d.Hourly {
			desc := firstDesc(h.Desc)
			out = append(out, Hourly{Time: d.Date + "T" + padTime(h.Time), TempC: parseFloat(h.Temp), PrecipProb: parsePtr(h.Chance), PrecipMm: parsePtr(h.Precip), WindKph: parsePtr(h.Wind), Icon: iconFromText(desc)})
			if len(out) >= 18 {
				return out
			}
		}
	}
	return out
}
func dayDesc(d wttrDay) string {
	if len(d.Hourly) > 0 {
		return firstDesc(d.Hourly[len(d.Hourly)/2].Desc)
	}
	return "Unknown"
}
func maxRainProb(hours []wttrHour) *float64 {
	var best float64
	found := false
	for _, h := range hours {
		p := parseFloat(h.Chance)
		if p > best {
			best = p
		}
		if h.Chance != "" {
			found = true
		}
	}
	if !found {
		return nil
	}
	return &best
}

func fetchMET(ctx context.Context, loc Location, contact string) (Current, []Hourly, []Daily, error) {
	endpoint := fmt.Sprintf("https://api.met.no/weatherapi/locationforecast/2.0/compact?lat=%.5f&lon=%.5f", loc.Latitude, loc.Longitude)
	ua := appVersion
	if strings.TrimSpace(contact) != "" {
		ua += " (" + strings.TrimSpace(contact) + ")"
	}
	var p metResponse
	if err := getJSON(ctx, endpoint, "User-Agent", ua, &p); err != nil {
		return Current{}, nil, nil, err
	}
	if len(p.Properties.Timeseries) == 0 {
		return Current{}, nil, nil, errors.New("MET Norway returned no forecast timeseries")
	}
	first := p.Properties.Timeseries[0]
	curDetails := first.Data.Instant.Details
	cur := Current{TempC: curDetails.AirTemperature, FeelsC: curDetails.AirTemperature, Humidity: ptr(curDetails.RelativeHumidity), WindKph: ptr(curDetails.WindSpeed * 3.6), PressureHpa: ptr(curDetails.SeaLevelPressure), CloudPct: ptr(curDetails.CloudFraction)}
	symbol := firstSymbol(first)
	cur.Condition = metSymbolText(symbol)
	cur.Icon = metSymbolIcon(symbol)
	hourly := make([]Hourly, 0, 18)
	for _, ts := range p.Properties.Timeseries[:minInt(18, len(p.Properties.Timeseries))] {
		sym := firstSymbol(ts)
		pr := ts.Data.Next1h.Details.Probability
		if pr == 0 {
			pr = ts.Data.Next6h.Details.Probability
		}
		hourly = append(hourly, Hourly{Time: ts.Time, TempC: ts.Data.Instant.Details.AirTemperature, PrecipProb: ptr(pr), PrecipMm: ptr(ts.Data.Next1h.Details.Precipitation), WindKph: ptr(ts.Data.Instant.Details.WindSpeed * 3.6), Icon: metSymbolIcon(sym)})
	}
	dailyMap := map[string]*Daily{}
	for _, ts := range p.Properties.Timeseries {
		date := ts.Time[:10]
		sym := firstSymbol(ts)
		temp := ts.Data.Instant.Details.AirTemperature
		d := dailyMap[date]
		if d == nil {
			d = &Daily{Date: date, Condition: metSymbolText(sym), Icon: metSymbolIcon(sym), MaxC: temp, MinC: temp}
			dailyMap[date] = d
		}
		if temp > d.MaxC {
			d.MaxC = temp
		}
		if temp < d.MinC {
			d.MinC = temp
		}
		pr := ts.Data.Next1h.Details.Probability
		if pr > d.PrecipProbVal() {
			d.PrecipProb = ptr(pr)
		}
	}
	dates := sortedDates(dailyMap)
	daily := make([]Daily, 0, minInt(7, len(dates)))
	for _, date := range dates {
		daily = append(daily, *dailyMap[date])
		if len(daily) == 7 {
			break
		}
	}
	return cur, hourly, daily, nil
}

func (d *Daily) PrecipProbVal() float64 {
	if d.PrecipProb == nil {
		return 0
	}
	return *d.PrecipProb
}

type metResponse struct {
	Properties struct {
		Timeseries []metTime `json:"timeseries"`
	} `json:"properties"`
}
type metTime struct {
	Time string `json:"time"`
	Data struct {
		Instant struct {
			Details struct {
				AirTemperature   float64 `json:"air_temperature"`
				RelativeHumidity float64 `json:"relative_humidity"`
				WindSpeed        float64 `json:"wind_speed"`
				SeaLevelPressure float64 `json:"air_pressure_at_sea_level"`
				CloudFraction    float64 `json:"cloud_area_fraction"`
			} `json:"details"`
		} `json:"instant"`
		Next1h struct {
			Summary struct {
				Symbol string `json:"symbol_code"`
			} `json:"summary"`
			Details struct {
				Probability   float64 `json:"probability_of_precipitation"`
				Precipitation float64 `json:"precipitation_amount"`
			} `json:"details"`
		} `json:"next_1_hours"`
		Next6h struct {
			Summary struct {
				Symbol string `json:"symbol_code"`
			} `json:"summary"`
			Details struct {
				Probability float64 `json:"probability_of_precipitation"`
			} `json:"details"`
		} `json:"next_6_hours"`
	} `json:"data"`
}

func firstSymbol(ts metTime) string {
	if ts.Data.Next1h.Summary.Symbol != "" {
		return ts.Data.Next1h.Summary.Symbol
	}
	if ts.Data.Next6h.Summary.Symbol != "" {
		return ts.Data.Next6h.Summary.Symbol
	}
	return "clearsky_day"
}
func metSymbolText(s string) string {
	s = strings.TrimSuffix(strings.TrimSuffix(strings.TrimSuffix(s, "_day"), "_night"), "_polartwilight")
	return strings.ReplaceAll(s, "_", " ")
}
func metSymbolIcon(s string) string {
	s = strings.ToLower(s)
	switch {
	case strings.Contains(s, "thunder"):
		return "storm"
	case strings.Contains(s, "snow") || strings.Contains(s, "sleet"):
		return "snow"
	case strings.Contains(s, "rain") || strings.Contains(s, "drizzle"):
		return "rain"
	case strings.Contains(s, "fog"):
		return "fog"
	case strings.Contains(s, "cloud"):
		return "partly"
	default:
		return "sun"
	}
}

func mergeReport(loc Location, results []sourceResult) (Report, error) {
	good := make([]sourceResult, 0, 3)
	statuses := make([]SourceStatus, 0, 3)
	for _, r := range results {
		note := ""
		if r.err != nil {
			note = r.err.Error()
		}
		statuses = append(statuses, SourceStatus{Name: r.name, OK: r.err == nil, MS: r.ms, Note: note})
		if r.err == nil {
			good = append(good, r)
		}
	}
	if len(good) == 0 {
		return Report{}, errors.New("All public weather sources failed. Try again in a moment.")
	}
	primary := good[0]
	for _, r := range good {
		if r.name == "Open-Meteo" {
			primary = r
			break
		}
	}
	temps := make([]float64, 0, len(good))
	for _, r := range good {
		temps = append(temps, r.current.TempC)
	}
	spread := maxFloat(temps) - minFloat(temps)
	msg := fmt.Sprintf("%d public source%s responded", len(good), plural(len(good)))
	if spread < 2 {
		msg += " and current temperatures are closely aligned"
	} else {
		msg += fmt.Sprintf("; current temperatures differ by %.1f°C", spread)
	}
	return Report{Location: loc, FetchedAt: time.Now().Local().Format("Jan 2, 2006, 3:04 PM"), Current: primary.current, Hourly: primary.hourly, Daily: primary.daily, Sources: statuses, Consensus: Consensus{SourceCount: len(good), TemperatureSpreadC: &spread, Message: msg}}, nil
}

func getJSON(ctx context.Context, endpoint, header, value string, dst any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return err
	}
	req.Header.Set(header, value)
	req.Header.Set("Accept", "application/json")
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	body, err := io.ReadAll(io.LimitReader(res.Body, 12<<20))
	if err != nil {
		return err
	}
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		return fmt.Errorf("HTTP %d from %s: %s", res.StatusCode, endpoint, httpStatusText(body))
	}
	if err := json.Unmarshal(body, dst); err != nil {
		return fmt.Errorf("invalid JSON from %s: %w", endpoint, err)
	}
	return nil
}
func httpStatusText(b []byte) string {
	s := strings.TrimSpace(string(b))
	if len(s) > 240 {
		s = s[:240]
	}
	return s
}

func cachePath(loc Location) string {
	dir, _ := os.UserCacheDir()
	if dir == "" {
		dir = os.TempDir()
	}
	dir = filepath.Join(dir, "sunbeam-weather")
	_ = os.MkdirAll(dir, 0755)
	return filepath.Join(dir, fmt.Sprintf("weather_%.3f_%.3f.json", loc.Latitude, loc.Longitude))
}
func readCache(loc Location) (Report, bool) {
	p := cachePath(loc)
	st, err := os.Stat(p)
	if err != nil || time.Since(st.ModTime()) > cacheTTL {
		return Report{}, false
	}
	b, err := os.ReadFile(p)
	if err != nil {
		return Report{}, false
	}
	var r Report
	if json.Unmarshal(b, &r) != nil {
		return Report{}, false
	}
	return r, true
}
func writeCache(loc Location, r Report) {
	b, err := json.Marshal(r)
	if err == nil {
		_ = os.WriteFile(cachePath(loc), b, 0644)
	}
}
func writeJSON(v any) { b, _ := json.MarshalIndent(v, "", "  "); fmt.Println(string(b)) }
func die(msg string)  { fmt.Fprintln(os.Stderr, msg); os.Exit(1) }
func minInt(a, b int) int {
	if a < b {
		return a
	}
	return b
}
func ptr[T any](v T) *T           { return &v }
func parseFloat(s string) float64 { v, _ := strconv.ParseFloat(strings.TrimSpace(s), 64); return v }
func parsePtr(s string) *float64 {
	if strings.TrimSpace(s) == "" {
		return nil
	}
	v := parseFloat(s)
	return &v
}
func fmtFloat(v float64) string { return strconv.Itoa(int(v + 0.5)) }
func firstDesc(v []struct {
	Value string `json:"value"`
}) string {
	if len(v) > 0 {
		return v[0].Value
	}
	return "Unknown"
}
func iconFromText(t string) string {
	t = strings.ToLower(t)
	switch {
	case strings.Contains(t, "thunder"):
		return "storm"
	case strings.Contains(t, "snow") || strings.Contains(t, "sleet"):
		return "snow"
	case strings.Contains(t, "rain") || strings.Contains(t, "drizzle"):
		return "rain"
	case strings.Contains(t, "fog") || strings.Contains(t, "mist"):
		return "fog"
	case strings.Contains(t, "overcast"):
		return "cloud"
	case strings.Contains(t, "cloud"):
		return "partly"
	default:
		return "sun"
	}
}
func weatherCode(code int, day bool) (string, string) {
	switch code {
	case 0:
		if !day {
			return "clear sky", "moon"
		}
		return "clear sky", "sun"
	case 1:
		if !day {
			return "mainly clear", "moon"
		}
		return "mainly clear", "sun"
	case 2:
		return "partly cloudy", "partly"
	case 3:
		return "overcast", "cloud"
	case 45, 48:
		return "fog", "fog"
	case 51, 53, 55, 56, 57:
		return "drizzle", "rain"
	case 61, 63, 65, 66, 67, 80, 81, 82:
		return "rain", "rain"
	case 71, 73, 75, 77, 85, 86:
		return "snow", "snow"
	case 95, 96, 99:
		return "thunderstorm", "storm"
	default:
		if !day {
			return "clear night", "moon"
		}
		return "weather", "cloud"
	}
}
func isDayHour(s string) bool {
	if len(s) < 13 {
		return true
	}
	h, _ := strconv.Atoi(s[11:13])
	return h >= 7 && h < 20
}
func padTime(s string) string {
	v, _ := strconv.Atoi(strings.TrimSuffix(s, "00"))
	return fmt.Sprintf("%02d:00", v)
}
func plural(n int) string {
	if n == 1 {
		return ""
	}
	return "s"
}
func maxFloat(v []float64) float64 {
	m := v[0]
	for _, x := range v {
		if x > m {
			m = x
		}
	}
	return m
}
func minFloat(v []float64) float64 {
	m := v[0]
	for _, x := range v {
		if x < m {
			m = x
		}
	}
	return m
}
func sortedDates(m map[string]*Daily) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	for i := 1; i < len(out); i++ {
		for j := i; j > 0 && out[j] < out[j-1]; j-- {
			out[j], out[j-1] = out[j-1], out[j]
		}
	}
	return out
}
