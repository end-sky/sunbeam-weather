#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, process::Command, sync::mpsc::{self, Receiver}, thread};

const APP_NAME: &str = "Sunbeam Weather";
const ACCENT_LIGHT: Color32 = Color32::from_rgb(26, 115, 232);
const ACCENT_DARK: Color32 = Color32::from_rgb(138, 180, 248);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Location {
    id: i64,
    name: String,
    admin1: Option<String>,
    country: Option<String>,
    latitude: f64,
    longitude: f64,
    timezone: Option<String>,
}

impl Location {
    fn label(&self) -> String {
        let mut pieces = vec![self.name.clone()];
        if let Some(admin) = self.admin1.as_deref().filter(|v| !v.is_empty()) {
            pieces.push(admin.to_owned());
        }
        if let Some(country) = self.country.as_deref().filter(|v| !v.is_empty()) {
            pieces.push(country.to_owned());
        }
        pieces.join(", ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Current {
    temp_c: f64,
    feels_c: f64,
    humidity: Option<f64>,
    wind_kph: Option<f64>,
    wind_dir: Option<String>,
    pressure_hpa: Option<f64>,
    visibility_km: Option<f64>,
    uv: Option<f64>,
    precip_mm: Option<f64>,
    cloud_pct: Option<f64>,
    condition: String,
    icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Hourly {
    time: String,
    temp_c: f64,
    precip_prob: Option<f64>,
    precip_mm: Option<f64>,
    wind_kph: Option<f64>,
    icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Daily {
    date: String,
    condition: String,
    icon: String,
    max_c: f64,
    min_c: f64,
    precip_prob: Option<f64>,
    sunrise: Option<String>,
    sunset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SourceStatus {
    name: String,
    ok: bool,
    ms: u64,
    note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Consensus {
    source_count: usize,
    temperature_spread_c: Option<f64>,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Report {
    location: Location,
    fetched_at: String,
    current: Current,
    hourly: Vec<Hourly>,
    daily: Vec<Daily>,
    sources: Vec<SourceStatus>,
    consensus: Consensus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Units { C, F }

impl Default for Units { fn default() -> Self { Self::C } }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Config {
    dark: bool,
    units: Units,
    met_contact: String,
    recent: Vec<Location>,
}

#[derive(Debug)]
enum WorkerResult {
    Search(Result<Vec<Location>, String>),
    Weather(Result<Report, String>),
}

struct WeatherApp {
    config: Config,
    query: String,
    locations: Vec<Location>,
    report: Option<Report>,
    loading: bool,
    error: Option<String>,
    status_text: String,
    worker: Option<Receiver<WorkerResult>>,
    show_settings: bool,
}

impl WeatherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = load_config();
        let app = Self {
            config,
            query: String::new(),
            locations: Vec::new(),
            report: None,
            loading: false,
            error: None,
            status_text: String::new(),
            worker: None,
            show_settings: false,
        };
        app.apply_theme(&cc.egui_ctx);
        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let theme = egui::Theme::from_dark_mode(self.config.dark);
        let accent = self.accent();
        let dark = self.config.dark;
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(14.0, 9.0);
            style.visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
            style.visuals.window_corner_radius = egui::CornerRadius::same(18);
            style.visuals.menu_corner_radius = egui::CornerRadius::same(16);
            style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(12);
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(12);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(12);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(12);
            style.visuals.widgets.open.corner_radius = egui::CornerRadius::same(12);
            style.visuals.selection.bg_fill = accent;
            style.visuals.hyperlink_color = accent;
        });
        ctx.set_theme(theme);
    }

    fn accent(&self) -> Color32 { if self.config.dark { ACCENT_DARK } else { ACCENT_LIGHT } }
    fn bg(&self) -> Color32 { if self.config.dark { Color32::from_rgb(16, 17, 20) } else { Color32::from_rgb(255, 255, 255) } }
    fn card(&self) -> Color32 { if self.config.dark { Color32::from_rgb(24, 26, 31) } else { Color32::from_rgb(255, 255, 255) } }
    fn soft(&self) -> Color32 { if self.config.dark { Color32::from_rgb(31, 34, 40) } else { Color32::from_rgb(248, 249, 250) } }
    fn text(&self) -> Color32 { if self.config.dark { Color32::from_rgb(241, 243, 244) } else { Color32::from_rgb(32, 33, 36) } }
    fn muted(&self) -> Color32 { if self.config.dark { Color32::from_rgb(165, 169, 176) } else { Color32::from_rgb(95, 99, 104) } }
    fn line(&self) -> Color32 { if self.config.dark { Color32::from_rgb(52, 55, 61) } else { Color32::from_rgb(223, 225, 229) } }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.config.dark = !self.config.dark;
        save_config(&self.config);
        self.apply_theme(ctx);
    }

    fn submit_search(&mut self) {
        let query = self.query.trim().to_owned();
        if query.is_empty() || self.loading { return; }
        self.loading = true;
        self.report = None;
        self.error = None;
        self.locations.clear();
        self.status_text = format!("Searching for {query}…");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = run_backend(&["search", &query]);
            let parsed = result.and_then(|s| serde_json::from_str::<Vec<Location>>(&s).map_err(|e| e.to_string()));
            let _ = tx.send(WorkerResult::Search(parsed));
        });
        self.worker = Some(rx);
    }

    fn fetch_weather(&mut self, loc: Location) {
        self.loading = true;
        self.error = None;
        self.report = None;
        self.status_text = format!("Checking three public weather sources for {}…", loc.label());
        self.remember_location(loc.clone());
        let contact = self.config.met_contact.clone();
        let units = match self.config.units { Units::C => "c", Units::F => "f" };
        let lat = loc.latitude.to_string();
        let lon = loc.longitude.to_string();
        let label = loc.label();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let args = ["weather", &lat, &lon, &label, units, &contact];
            let result = run_backend(&args).and_then(|s| serde_json::from_str::<Report>(&s).map_err(|e| e.to_string()));
            let _ = tx.send(WorkerResult::Weather(result));
        });
        self.worker = Some(rx);
    }

    fn remember_location(&mut self, loc: Location) {
        self.config.recent.retain(|x| (x.latitude - loc.latitude).abs() > 0.001 || (x.longitude - loc.longitude).abs() > 0.001);
        self.config.recent.insert(0, loc);
        self.config.recent.truncate(5);
        save_config(&self.config);
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let mut done = None;
        if let Some(rx) = &self.worker {
            if let Ok(msg) = rx.try_recv() { done = Some(msg); }
        }
        if let Some(msg) = done {
            self.worker = None;
            self.loading = false;
            match msg {
                WorkerResult::Search(result) => match result {
                    Ok(locations) if locations.is_empty() => self.error = Some("No places found. Try a city, region, or country.".into()),
                    Ok(locations) => {
                        self.status_text.clear();
                        self.locations = locations;
                        if let Some(loc) = self.locations.first().cloned() { self.fetch_weather(loc); }
                    }
                    Err(err) => self.error = Some(err),
                },
                WorkerResult::Weather(result) => match result {
                    Ok(report) => { self.status_text.clear(); self.report = Some(report); }
                    Err(err) => self.error = Some(err),
                },
            }
            ctx.request_repaint();
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let label = egui::Button::new(RichText::new("Sunbeam").size(18.0).strong().color(self.text()))
                    .frame(false);
                if ui.add(label).clicked() {
                    self.report = None;
                    self.locations.clear();
                    self.error = None;
                    self.status_text.clear();
                }
                ui.label(RichText::new("Weather").size(18.0).color(self.muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let theme_icon = if self.config.dark { "☾" } else { "☀" };
                let response = ui.add_sized([42.0, 42.0], egui::Button::new(RichText::new(theme_icon).size(22.0)).frame(false));
                if response.on_hover_text(if self.config.dark { "Switch to light mode" } else { "Switch to dark mode" }).clicked() {
                    self.toggle_theme(ctx);
                }
                let settings = ui.add_sized([42.0, 42.0], egui::Button::new(RichText::new("⚙").size(20.0)).frame(false));
                if settings.on_hover_text("Settings").clicked() { self.show_settings = true; }
            });
        });
        ui.add_space(6.0);
    }

    fn search_bar(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new()
            .fill(self.card())
            .stroke(Stroke::new(1.0, self.line()))
            .corner_radius(egui::CornerRadius::same(18))
            .inner_margin(egui::Margin::symmetric(12, 10));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⌕").size(25.0).color(self.muted()));
                let input = ui.add_sized([ui.available_width() - 98.0, 36.0], egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search any city, region or country")
                    .font(FontId::proportional(17.0)));
                if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { self.submit_search(); }
                let btn = ui.add_sized([76.0, 36.0], egui::Button::new(if self.loading { "…" } else { "Search" }).fill(self.text()).corner_radius(egui::CornerRadius::same(11)));
                if btn.clicked() { self.submit_search(); }
            });
        });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("A clearer view of the weather.").size(32.0).strong().color(self.text()));
            ui.add_space(8.0);
            ui.label(RichText::new("One search, multiple independent public sources, one calm answer.").size(16.0).color(self.muted()));
            ui.add_space(24.0);
        });
        self.search_bar(ui);
        if !self.config.recent.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Recent").small().color(self.muted()));
                for loc in self.config.recent.clone() {
                    let text = loc.name.clone();
                    if ui.add(egui::Button::new(text).fill(self.soft()).corner_radius(egui::CornerRadius::same(18))).clicked() {
                        self.query = loc.name.clone();
                        self.fetch_weather(loc);
                    }
                }
            });
        }
        ui.add_space(28.0);
        let card = egui::Frame::new().fill(self.soft()).corner_radius(egui::CornerRadius::same(18)).inner_margin(egui::Margin::same(20));
        card.show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Try").strong().color(self.text()));
                for sample in ["Dallas", "London", "Tokyo", "Reykjavík"] {
                    if ui.link(sample).clicked() {
                        self.query = sample.into();
                        self.submit_search();
                    }
                }
            });
            ui.add_space(12.0);
            ui.label(RichText::new("No account. No API keys. Weather data comes from public endpoints.").small().color(self.muted()));
        });
    }

    fn result_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(report) = self.report.clone() else { return; };
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(report.location.label()).size(27.0).strong().color(self.text()));
                ui.label(RichText::new(format!("{} · {:.2}, {:.2}", report.location.timezone.clone().unwrap_or_else(|| "Local time".into()), report.location.latitude, report.location.longitude)).size(13.0).color(self.muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("↻ Refresh").clicked() { self.fetch_weather(report.location.clone()); }
                if ui.button("⌂ Home").clicked() { self.report = None; self.error = None; }
            });
        });
        ui.add_space(16.0);
        if self.locations.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Other matches").small().color(self.muted()));
                for loc in self.locations.clone().into_iter().take(5) {
                    if ui.add(egui::Button::new(loc.label()).fill(self.soft()).corner_radius(egui::CornerRadius::same(16))).clicked() {
                        self.fetch_weather(loc);
                    }
                }
            });
            ui.add_space(10.0);
        }

        let columns = if ui.available_width() > 920.0 { 2 } else { 1 };
        if columns == 2 {
            ui.columns(2, |cols| {
                current_card(self, &mut cols[0], &report);
                daily_card(self, &mut cols[1], &report);
            });
        } else {
            current_card(self, ui, &report);
            ui.add_space(12.0);
            daily_card(self, ui, &report);
        }

        ui.add_space(12.0);
        hourly_card(self, ui, &report);
        ui.add_space(12.0);
        details_card(self, ui, &report);
        ui.add_space(12.0);
        source_card(self, ui, &report);
        ui.add_space(22.0);
        ui.label(RichText::new(format!("Updated {} · Public sources · No sign-in required", report.fetched_at)).small().color(self.muted()));
        ctx.request_repaint_after(std::time::Duration::from_secs(30));
    }

    fn settings(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Display").strong().size(17.0));
                ui.add_space(8.0);
                egui::ComboBox::from_label("Temperature units")
                    .selected_text(match self.config.units { Units::C => "Celsius (°C)", Units::F => "Fahrenheit (°F)" })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(matches!(self.config.units, Units::C), "Celsius (°C)").clicked() { self.config.units = Units::C; }
                        if ui.selectable_label(matches!(self.config.units, Units::F), "Fahrenheit (°F)").clicked() { self.config.units = Units::F; }
                    });
                ui.add_space(18.0);
                ui.label(RichText::new("MET Norway contact").strong().size(17.0));
                ui.label(RichText::new("MET Norway asks clients to identify themselves with a meaningful User-Agent. Add a site or contact address; this is stored locally.").small().color(self.muted()));
                ui.add_space(8.0);
                ui.add(egui::TextEdit::singleline(&mut self.config.met_contact).hint_text("https://your-site.example or you@example.org"));
                ui.add_space(16.0);
                if ui.button("Save settings").clicked() { save_config(&self.config); self.show_settings = false; }
            });
        if !open { self.show_settings = false; }
    }
}

impl eframe::App for WeatherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_worker(&ctx);
        if self.show_settings { self.settings(&ctx); }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(self.bg()).inner_margin(egui::Margin::symmetric(30, 18))).show(ui, |ui| {
            let max = 1080.0_f32.min(ui.available_width());
            ui.set_max_width(max);
            self.top_bar(ui, &ctx);
            if self.report.is_none() && !self.loading && self.error.is_none() {
                self.home(ui);
            } else {
                self.search_bar(ui);
            }
            if self.loading {
                ui.add_space(12.0);
                egui::Frame::new().fill(self.soft()).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::same(13)).show(ui, |ui| {
                    ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new(&self.status_text).color(self.muted())); });
                });
            }
            if let Some(error) = &self.error {
                ui.add_space(12.0);
                egui::Frame::new().fill(Color32::from_rgba_unmultiplied(219, 68, 55, if self.config.dark { 35 } else { 20 })).stroke(Stroke::new(1.0, Color32::from_rgb(219, 68, 55))).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("!  ").strong().color(Color32::from_rgb(219, 68, 55)));
                        ui.label(RichText::new(error).color(self.text()));
                    });
                });
            }
            if self.report.is_some() && !self.loading { self.result_view(ui, &ctx); }
        });
    }
}

fn current_card(app: &WeatherApp, ui: &mut egui::Ui, report: &Report) {
    card_frame(app).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(&report.current.icon).size(56.0));
                ui.add_space(4.0);
                ui.label(RichText::new(display_temp(app, report.current.temp_c)).size(58.0).strong().color(app.text()));
                ui.label(RichText::new(title_case(&report.current.condition)).size(18.0).color(app.text()));
                ui.label(RichText::new(format!("Feels like {}", display_temp(app, report.current.feels_c))).small().color(app.muted()));
            });
            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                let status = if report.consensus.source_count >= 3 { "High agreement" } else if report.consensus.source_count == 2 { "Good agreement" } else { "Single source" };
                ui.label(RichText::new(status).small().color(app.accent()));
            });
        });
        ui.add_space(18.0);
        ui.horizontal_wrapped(|ui| {
            metric(app, ui, "Humidity", report.current.humidity.map(|v| format!("{v:.0}%")));
            metric(app, ui, "Wind", report.current.wind_kph.map(|v| format!("{v:.0} km/h {}", report.current.wind_dir.clone().unwrap_or_default())));
            metric(app, ui, "UV", report.current.uv.map(|v| format!("{v:.0}")));
            metric(app, ui, "Pressure", report.current.pressure_hpa.map(|v| format!("{v:.0} hPa")));
        });
    });
}

fn daily_card(app: &WeatherApp, ui: &mut egui::Ui, report: &Report) {
    card_frame(app).show(ui, |ui| {
        ui.label(RichText::new("7-day forecast").size(16.0).strong().color(app.text()));
        ui.add_space(12.0);
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (index, day) in report.daily.iter().enumerate() {
                    egui::Frame::new().fill(if index == 0 { app.soft() } else { app.card() }).corner_radius(egui::CornerRadius::same(15)).inner_margin(egui::Margin::symmetric(11, 12)).show(ui, |ui| {
                        ui.set_min_width(95.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(if index == 0 { "Today".into() } else { weekday(&day.date) }).strong().color(app.text()));
                            ui.label(RichText::new(day.icon.clone()).size(30.0));
                            ui.label(RichText::new(format!("{} / {}", display_temp(app, day.max_c), display_temp(app, day.min_c))).size(15.0).strong().color(app.text()));
                            if let Some(p) = day.precip_prob { ui.label(RichText::new(format!("{p:.0}% rain")).small().color(app.accent())); }
                        });
                    });
                }
            });
        });
    });
}

fn hourly_card(app: &WeatherApp, ui: &mut egui::Ui, report: &Report) {
    card_frame(app).show(ui, |ui| {
        ui.label(RichText::new("Next hours").size(16.0).strong().color(app.text()));
        ui.add_space(8.0);
        draw_temp_chart(app, ui, &report.hourly);
        ui.add_space(8.0);
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for hour in report.hourly.iter().take(16) {
                    let h = format_hour(&hour.time);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(9, 8)).show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(h).small().color(app.muted()));
                            ui.label(RichText::new(hour.icon.clone()).size(25.0));
                            ui.label(RichText::new(display_temp(app, hour.temp_c)).strong().color(app.text()));
                            if let Some(p) = hour.precip_prob { ui.label(RichText::new(format!("{p:.0}%")).small().color(app.accent())); }
                        });
                    });
                }
            });
        });
    });
}

fn draw_temp_chart(app: &WeatherApp, ui: &mut egui::Ui, hours: &[Hourly]) {
    let h = 150.0;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, egui::CornerRadius::same(14), Stroke::new(1.0, app.line()), egui::StrokeKind::Inside);
    if hours.len() < 2 { return; }
    let min = hours.iter().map(|x| x.temp_c).fold(f64::INFINITY, f64::min);
    let max = hours.iter().map(|x| x.temp_c).fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1.0);
    let points: Vec<egui::Pos2> = hours.iter().enumerate().map(|(i, x)| {
        let px = rect.left() + rect.width() * (i as f32 / (hours.len().saturating_sub(1) as f32));
        let py = rect.bottom() - rect.height() * ((x.temp_c - min) / span) as f32 * 0.72 - 18.0;
        egui::pos2(px, py)
    }).collect();
    for i in 0..points.len() - 1 { painter.line_segment([points[i], points[i + 1]], Stroke::new(3.0, app.accent())); }
    for p in points.iter().step_by(3) { painter.circle_filled(*p, 4.0, app.accent()); }
    painter.text(rect.left_top() + Vec2::new(12.0, 10.0), egui::Align2::LEFT_TOP, display_temp(app, max), FontId::proportional(12.0), app.muted());
    painter.text(rect.left_bottom() + Vec2::new(12.0, -12.0), egui::Align2::LEFT_BOTTOM, display_temp(app, min), FontId::proportional(12.0), app.muted());
}

fn details_card(app: &WeatherApp, ui: &mut egui::Ui, report: &Report) {
    card_frame(app).show(ui, |ui| {
        ui.label(RichText::new("Details").size(16.0).strong().color(app.text()));
        ui.add_space(10.0);
        egui::Grid::new("detail_grid").num_columns(4).spacing(Vec2::new(28.0, 14.0)).striped(false).show(ui, |ui| {
            detail_item(app, ui, "Feels like", display_temp(app, report.current.feels_c));
            detail_item(app, ui, "Humidity", report.current.humidity.map_or("—".into(), |v| format!("{v:.0}%")));
            detail_item(app, ui, "Wind", report.current.wind_kph.map_or("—".into(), |v| format!("{v:.0} km/h {}", report.current.wind_dir.clone().unwrap_or_default())));
            detail_item(app, ui, "Pressure", report.current.pressure_hpa.map_or("—".into(), |v| format!("{v:.0} hPa")));
            ui.end_row();
            detail_item(app, ui, "Visibility", report.current.visibility_km.map_or("—".into(), |v| format!("{v:.1} km")));
            detail_item(app, ui, "Cloud cover", report.current.cloud_pct.map_or("—".into(), |v| format!("{v:.0}%")));
            detail_item(app, ui, "Precipitation", report.current.precip_mm.map_or("—".into(), |v| format!("{v:.1} mm")));
            detail_item(app, ui, "UV index", report.current.uv.map_or("—".into(), |v| format!("{v:.0}")));
        });
    });
}

fn source_card(app: &WeatherApp, ui: &mut egui::Ui, report: &Report) {
    card_frame(app).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Source agreement").size(16.0).strong().color(app.text()));
            ui.add_space(6.0);
            ui.label(RichText::new(&report.consensus.message).small().color(app.muted()));
        });
        ui.add_space(9.0);
        ui.horizontal_wrapped(|ui| {
            for src in &report.sources {
                let mark = if src.ok { "●" } else { "○" };
                let fill = if src.ok { app.soft() } else { app.card() };
                let color = if src.ok { app.accent() } else { app.muted() };
                egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, app.line())).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                    ui.horizontal(|ui| { ui.label(RichText::new(mark).color(color)); ui.label(RichText::new(&src.name).color(app.text())); ui.label(RichText::new(format!("{} ms", src.ms)).small().color(app.muted())); });
                    if !src.note.is_empty() { ui.label(RichText::new(&src.note).small().color(app.muted())); }
                });
            }
        });
        if let Some(spread) = report.consensus.temperature_spread_c { ui.add_space(8.0); ui.label(RichText::new(format!("Current-temperature spread across successful sources: {spread:.1}°C")).small().color(app.muted())); }
    });
}

fn card_frame(app: &WeatherApp) -> egui::Frame {
    egui::Frame::new().fill(app.card()).stroke(Stroke::new(1.0, app.line())).corner_radius(egui::CornerRadius::same(18)).inner_margin(egui::Margin::same(18))
}

fn metric(app: &WeatherApp, ui: &mut egui::Ui, label: &str, value: Option<String>) {
    let value = value.unwrap_or_else(|| "—".into());
    egui::Frame::new().fill(app.soft()).corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::symmetric(11, 9)).show(ui, |ui| {
        ui.vertical(|ui| { ui.label(RichText::new(label).small().color(app.muted())); ui.label(RichText::new(value).strong().color(app.text())); });
    });
}

fn detail_item(app: &WeatherApp, ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(RichText::new(label).small().color(app.muted()));
    ui.label(RichText::new(value).strong().color(app.text()));
}

fn display_temp(app: &WeatherApp, c: f64) -> String {
    match app.config.units { Units::C => format!("{:.0}°", c), Units::F => format!("{:.0}°", c * 9.0 / 5.0 + 32.0) }
}

fn weekday(date: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 { return date.to_owned(); }
    let y: i32 = parts[0].parse().unwrap_or(2026); let m: u32 = parts[1].parse().unwrap_or(1); let d: u32 = parts[2].parse().unwrap_or(1);
    // Sakamoto weekday; returns 0=Sunday.
    let mut yy = y;
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if m < 3 { yy -= 1; }
    let w = (yy + yy/4 - yy/100 + yy/400 + t[(m - 1) as usize] + d as i32) % 7;
    ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][w as usize].to_owned()
}

fn format_hour(value: &str) -> String {
    value.split('T').nth(1).and_then(|t| t.get(0..5)).unwrap_or(value).to_owned()
}

fn title_case(value: &str) -> String {
    value.split_whitespace().map(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => {
                let first: String = first.to_uppercase().collect();
                format!("{}{}", first, chars.as_str().to_lowercase())
            }
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

fn run_backend(args: &[&str]) -> Result<String, String> {
    let exe = backend_path();
    let mut cmd = Command::new(&exe);
    cmd.args(args);
    let output = cmd.output().map_err(|e| format!("Could not start weather backend ({}): {e}", exe.display()))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| format!("Backend returned invalid UTF-8: {e}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() { format!("Weather backend exited with {}", output.status) } else { stderr })
    }
}

fn backend_path() -> PathBuf {
    if let Ok(custom) = std::env::var("SUNBEAM_WEATHER_BACKEND") { return PathBuf::from(custom); }
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
    if let Some(dir) = exe_dir {
        let candidate = dir.join("weather-scraper");
        if candidate.exists() { return candidate; }
    }
    PathBuf::from("weather-scraper")
}

fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") { return PathBuf::from(dir).join("sunbeam-weather").join("config.json"); }
    if let Ok(home) = std::env::var("HOME") { return PathBuf::from(home).join(".config/sunbeam-weather/config.json"); }
    PathBuf::from("config.json")
}

fn load_config() -> Config {
    fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_config(config: &Config) {
    let path = config_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    if let Ok(json) = serde_json::to_string_pretty(config) { let _ = fs::write(path, json); }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1220.0, 820.0]).with_min_inner_size([920.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(WeatherApp::new(cc)))))
}
