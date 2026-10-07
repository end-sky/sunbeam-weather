package main

import (
	"encoding/json"
	"math"
	"testing"
)

func TestWeatherCodeMap(t *testing.T) {
	cases := []struct {
		code               int
		wantText, wantIcon string
	}{
		{0, "clear sky", "sun"},
		{2, "partly cloudy", "partly"},
		{63, "rain", "rain"},
		{75, "snow", "snow"},
		{95, "thunderstorm", "storm"},
	}
	for _, tc := range cases {
		gotText, gotIcon := weatherCode(tc.code, true)
		if gotText != tc.wantText || gotIcon != tc.wantIcon {
			t.Fatalf("weatherCode(%d) = %q, %q; want %q, %q", tc.code, gotText, gotIcon, tc.wantText, tc.wantIcon)
		}
	}
}

func TestWttrNestedDataCompatibility(t *testing.T) {
	raw := []byte(`{
      "data": {
        "current_condition": [{"temp_C":"21","FeelsLikeC":"20","weatherDesc":[{"value":"Partly cloudy"}]}],
        "weather": [{"date":"2026-10-07","maxtempC":"24","mintempC":"16","hourly":[{"time":"1200","tempC":"21","chanceofrain":"10","weatherDesc":[{"value":"Partly cloudy"}]}]}]
      }
    }`)
	var env wttrEnvelope
	if err := json.Unmarshal(raw, &env); err != nil {
		t.Fatal(err)
	}
	if env.Current() == nil {
		t.Fatal("nested current_condition was not detected")
	}
	if len(env.Weather()) != 1 {
		t.Fatalf("nested weather length = %d; want 1", len(env.Weather()))
	}
	cur := normalizeWttrCurrent(*env.Current())
	if cur.TempC != 21 || cur.FeelsC != 20 || cur.Icon != "partly" {
		t.Fatalf("unexpected current normalization: %+v", cur)
	}
}

func TestMergeConsensus(t *testing.T) {
	results := []sourceResult{
		{name: "Open-Meteo", current: Current{TempC: 21.0}, ms: 90},
		{name: "wttr.in", current: Current{TempC: 21.7}, ms: 120},
		{name: "MET Norway", current: Current{TempC: 20.8}, ms: 150},
	}
	report, err := mergeReport(Location{Name: "Test", Latitude: 1, Longitude: 2}, results)
	if err != nil {
		t.Fatal(err)
	}
	if report.Consensus.SourceCount != 3 {
		t.Fatalf("source count = %d", report.Consensus.SourceCount)
	}
	if report.Consensus.TemperatureSpreadC == nil || math.Abs(*report.Consensus.TemperatureSpreadC-0.9) > 0.01 {
		t.Fatalf("spread = %v", report.Consensus.TemperatureSpreadC)
	}
	if report.Current.TempC != 21.0 {
		t.Fatalf("primary temp = %v; want Open-Meteo value", report.Current.TempC)
	}
}
