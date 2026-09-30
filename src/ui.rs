use crate::engine::fx::FxId;
use crate::engine::{Command, Engine, Snapshot, DECKS, SCENES, TRACKS};
use crate::theme::Theme;
use eframe::egui::{
    self, Align, Color32, FontId, Key, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Instant, SystemTime};
use walkdir::WalkDir;

pub struct App {
    engine: Engine,
    theme: Theme,
    fonts_set: bool,
    library: Vec<LibItem>,
    lib_filter: String,
    lib_sel: usize,
    keys_open: bool,
    midi_open: bool,
    status: String,
    submission_error: Cell<Option<crate::engine::SubmissionError>>,
    seen_submission_failures: u64,
    last_theme_check: Instant,
    load_tx: mpsc::Sender<(u8, PathBuf)>,
    load_rx: mpsc::Receiver<(u8, Result<crate::engine::dsp::Sample, String>)>,
    snap: Snapshot,
    last_play_idx: usize,
    pad_held: [bool; 16],
}

struct LibItem {
    title: String,
    artist: String,
    bpm: f32,
    key: String,
    length: f32,
    last_play: Option<SystemTime>,
    path: PathBuf,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine) -> Self {
        Theme::install_fonts(&cc.egui_ctx);
        let theme = Theme::load();
        theme.apply(&cc.egui_ctx);
        let (tx, rx_paths) = mpsc::channel::<(u8, PathBuf)>();
        let (tx_done, rx_done) = mpsc::channel();
        std::thread::Builder::new()
            .name("omatainer-decode".into())
            .spawn(move || {
                while let Ok((deck, path)) = rx_paths.recv() {
                    let r = crate::engine::dsp::decode_audio(&path).map_err(|e| e.to_string());
                    let _ = tx_done.send((deck, r));
                }
            })
            .ok();
        let mut app = Self {
            engine,
            theme,
            fonts_set: true,
            library: Vec::new(),
            lib_filter: String::new(),
            lib_sel: 0,
            keys_open: false,
            midi_open: false,
            status: "Q quant · pads compose · ctrl-gain = fx".into(),
            submission_error: Cell::new(None),
            seen_submission_failures: 0,
            last_theme_check: Instant::now(),
            load_tx: tx,
            load_rx: rx_done,
            snap: Snapshot::default(),
            last_play_idx: 0,
            pad_held: [false; 16],
        };
        app.scan_library();
        app.snap = app.engine.snapshot();
        app
    }

    fn scan_library(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let mut items = vec![
            LibItem {
                title: "Drums (session)".into(),
                artist: "omatainer".into(),
                bpm: 124.0,
                key: "C".into(),
                length: 16.0 * 60.0 / 124.0,
                last_play: None,
                path: PathBuf::from("builtin:drums"),
            },
            LibItem {
                title: "Harmony (session)".into(),
                artist: "omatainer".into(),
                bpm: 124.0,
                key: "C".into(),
                length: 16.0 * 60.0 / 124.0,
                last_play: None,
                path: PathBuf::from("builtin:harmony"),
            },
        ];
        for root in [PathBuf::from(&home).join("Music"), PathBuf::from(&home).join("music")] {
            if !root.exists() {
                continue;
            }
            for e in WalkDir::new(&root).max_depth(6).into_iter().flatten() {
                let p = e.path();
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
                if !matches!(ext.as_str(), "wav" | "mp3" | "flac" | "ogg" | "aiff" | "aif" | "m4a" | "aac") {
                    continue;
                }
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
                let (artist, title) = split_artist_title(stem);
                let (bpm, key) = parse_tags(stem);
                items.push(LibItem {
                    title,
                    artist,
                    bpm,
                    key,
                    length: 0.0,
                    last_play: None,
                    path: p.to_path_buf(),
                });
            }
        }
        sort_crate(&mut items);
        self.library = items;
    }

    fn send(&self, c: Command) {
        self.submit(c);
    }

    fn submit(&self, c: Command) -> bool {
        match self.engine.send(c) {
            Ok(_) => true,
            Err(error) => {
                self.submission_error.set(Some(error));
                false
            }
        }
    }

    fn load_sel(&mut self, deck: u8) {
        let picked = self.filtered().get(self.lib_sel).map(|i| (i.title.clone(), i.path.clone()));
        if let Some((name, path)) = picked {
            self.last_play_idx = self.lib_sel;
            if let Some(it) = self.filtered_get_mut(self.lib_sel) {
                it.last_play = Some(SystemTime::now());
            }
            if path.starts_with("builtin:") {
                let stem = if path.to_string_lossy().contains("harmony") { 1u8 } else { 0 };
                if self.submit(Command::LoadBuiltin { deck, stem }) {
                    self.status = format!("queued {name} → {}", (b'A' + deck) as char);
                } else {
                    self.status = "Load was not accepted".into();
                }
                return;
            }
            self.status = format!("loading {name} → {}", (b'A' + deck) as char);
            let _ = self.load_tx.send((deck, path));
        }
    }

    fn pad_gate(&mut self, p: usize, enabled: bool, r: &egui::Response) {
        if !enabled || p >= 16 {
            return;
        }
        let down = r.is_pointer_button_down_on();
        if down && !self.pad_held[p] {
            self.pad_held[p] = true;
            self.send(Command::SamplerPad {
                pad: p as u8,
                on: true,
            });
        } else if !down && self.pad_held[p] {
            self.pad_held[p] = false;
            self.send(Command::SamplerPad {
                pad: p as u8,
                on: false,
            });
        }
    }

    fn filtered(&self) -> Vec<&LibItem> {
        let q = self.lib_filter.to_lowercase();
        self.library
            .iter()
            .filter(|i| {
                q.is_empty()
                    || i.title.to_lowercase().contains(&q)
                    || i.artist.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn filtered_get_mut(&mut self, i: usize) -> Option<&mut LibItem> {
        let path = self.filtered().get(i)?.path.clone();
        self.library.iter_mut().find(|x| x.path == path)
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let submissions = self.engine.cmd.stats();
        if submissions.rejected > self.seen_submission_failures {
            self.seen_submission_failures = submissions.rejected;
            self.submission_error.set(submissions.last_error);
        }
        if self.last_theme_check.elapsed().as_millis() > 800 {
            if self.theme.maybe_reload() {
                self.theme.apply(ctx);
            }
            self.last_theme_check = Instant::now();
        }
        if !self.fonts_set {
            Theme::install_fonts(ctx);
            self.fonts_set = true;
        }
        while let Ok((deck, res)) = self.load_rx.try_recv() {
            match res {
                Ok(s) => {
                    let queued = format!("queued {}  {:.1} bpm", s.name, s.bpm);
                    self.status = if self.submit(Command::DeckAudio {
                        deck,
                        audio: std::sync::Arc::new(s),
                    }) {
                        queued
                    } else {
                        "Load was not accepted".into()
                    };
                }
                Err(e) => self.status = format!("load failed: {e}"),
            }
        }
        self.snap = self.engine.snapshot();
        self.handle_keys(ctx);
        let animating = self.snap.playing || self.snap.decks.iter().any(|d| d.playing);
        if let Some(p) = ctx.input(|i| {
            i.raw.dropped_files.iter().find_map(|f| f.path.clone())
        }) {
            let x = ctx.input(|i| i.pointer.latest_pos().map(|p| p.x)).unwrap_or(0.0);
            let deck = if x > ctx.screen_rect().center().x { 1u8 } else { 0 };
            let _ = self.load_tx.send((deck, p));
        }

        let t = self.theme.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg).inner_margin(6.0))
            .show(ctx, |ui| {
                let h = ui.available_height();
                let gap = 4.0;
                let samp_h = 88.0;
                let crate_h = 108.0;
                let seq_row = (t.font_size + 10.0).clamp(20.0, 26.0);
                let seq_h = 26.0 + 48.0 + seq_row * SCENES as f32 + gap * (SCENES as f32 + 2.0);
                let scratch_h = (h - samp_h - crate_h - seq_h - gap * 3.0).max(200.0);
                ui.allocate_ui(Vec2::new(ui.available_width(), scratch_h), |ui| {
                    self.scratch_row(ui, &t);
                });
                ui.add_space(gap);
                ui.allocate_ui(Vec2::new(ui.available_width(), samp_h), |ui| {
                    self.sampler_row(ui, &t);
                });
                ui.add_space(gap);
                ui.allocate_ui(Vec2::new(ui.available_width(), crate_h), |ui| {
                    self.crate_row(ui, &t);
                });
                ui.add_space(gap);
                if self.snap.fx_view >= 0 {
                    self.fx_row(ui, &t);
                } else {
                    ui.allocate_ui(Vec2::new(ui.available_width(), seq_h), |ui| {
                        self.sequencer_row(ui, &t);
                    });
                }
            });

        if let Some(error) = self.submission_error.get() {
            egui::Window::new("Action was not accepted")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(error.to_string());
                    if ui.button("Dismiss").clicked() {
                        self.submission_error.set(None);
                    }
                });
        }
        if self.keys_open {
            egui::Window::new("keys").show(ctx, |ui| {
                ui.monospace(KEYS);
            });
        }
        if self.midi_open {
            egui::Window::new("midi").show(ctx, |ui| {
                for d in &self.snap.midi {
                    ui.label(d);
                }
            });
        }
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }
}

impl App {
    fn handle_keys(&mut self, ctx: &egui::Context) {
        let mods = ctx.input(|i| i.modifiers);
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, repeat, .. } = ev {
                    if *repeat {
                        continue;
                    }
                    match key {
                        Key::Space => self.send(Command::TogglePlay),
                        Key::Q if !mods.shift => self.send(Command::DeckPlay { deck: 0 }),
                        Key::P => self.send(Command::DeckPlay { deck: 1 }),
                        Key::A if !mods.ctrl => self.send(Command::DeckCue { deck: 0 }),
                        Key::L => self.send(Command::DeckCue { deck: 1 }),
                        Key::Slash if mods.shift => self.keys_open = !self.keys_open,
                        Key::F1 => self.keys_open = !self.keys_open,
                        Key::M if mods.ctrl => self.midi_open = !self.midi_open,
                        Key::Escape => self.send(Command::CloseFx),
                        _ => {}
                    }
                }
            }
        });
    }

    fn scratch_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.spacing_mut().item_spacing = Vec2::splat(4.0);
        let h = ui.available_height();
        let w = ui.available_width();
        let gap = 4.0;
        let fader_h = 28.0;
        let (sq, wave_h, side_w, mid_w) = scratch_metrics(w, h, fader_h, gap);
        ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(gap);
            ui.allocate_ui(Vec2::new(side_w, h), |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                self.deck_side(ui, t, 0, wave_h, fader_h, sq);
            });
            ui.allocate_ui(Vec2::new(mid_w, h), |ui| {
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(gap);
                    let wave_w = ((mid_w - gap) / 2.0).max(56.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        for d in 0..DECKS {
                            let snap = self.snap.decks.get(d).cloned().unwrap_or_default();
                            vertical_wave(ui, t, &snap, t.track_color(d), wave_w, wave_h, |frac| {
                                self.send(Command::DeckSeek { deck: d as u8, frac });
                            });
                        }
                    });
                    let mut x = self.snap.xfader;
                    if xfader(ui, t, mid_w, fader_h, &mut x) {
                        self.send(Command::Xfader(x));
                    }
                });
            });
            ui.allocate_ui(Vec2::new(side_w, h), |ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(gap);
                self.deck_side(ui, t, 1, wave_h, fader_h, sq);
            });
        });
    }

    fn deck_side(&mut self, ui: &mut Ui, t: &Theme, d: usize, wave_h: f32, fader_h: f32, sq: f32) {
        let snap = self.snap.decks.get(d).cloned().unwrap_or_default();
        let col = t.track_color(d);
        let h = wave_h + fader_h + 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_min_height(h);
            if d == 0 {
                self.speed_col(ui, t, d, &snap, h, sq);
                self.cue_eq_col(ui, t, d, &snap, col, wave_h, sq);
                self.platter_col(ui, t, d, &snap, col, wave_h);
                self.btn_stack(ui, t, d, &snap, sq);
            } else {
                self.btn_stack(ui, t, d, &snap, sq);
                self.platter_col(ui, t, d, &snap, col, wave_h);
                self.cue_eq_col(ui, t, d, &snap, col, wave_h, sq);
                self.speed_col(ui, t, d, &snap, h, sq);
            }
        });
    }

    fn speed_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, h: f32, sq: f32) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_width(sq);
            ui.set_min_height(h);
            if sq_btn(ui, t, "L", snap.keylock, t.cyan, sq).on_hover_text("pitch lock").clicked() {
                self.send(Command::DeckKeylock { deck: d as u8 });
            }
            let fader_h = (h - sq * 2.0 - 8.0).max(48.0);
            if let Some(v) = fader(ui, t, snap.pitch, 1.0, 0.0, t.accent, sq, fader_h) {
                self.send(Command::DeckPitch { deck: d as u8, value: v });
            }
            let lab = ["8", "16", "50"][snap.pitch_range.min(2) as usize];
            if sq_btn(ui, t, lab, false, t.orange, sq).on_hover_text("pitch range").clicked() {
                self.send(Command::DeckPitchRange { deck: d as u8 });
            }
        });
    }

    fn cue_eq_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, col: Color32, wave_h: f32, sq: f32) {
        let cell = sq;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(3.0);
            ui.set_width(cell * 4.0 + 9.0);
            ui.set_min_height(wave_h);
            for row in 0..2 {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                    for c in 0..4 {
                        let i = row * 4 + c;
                        let on = snap.hotcues.get(i).copied().unwrap_or(false);
                        let r = sq_btn(ui, t, &format!("{}", i + 1), on, t.track_color(i), cell);
                        if r.clicked() {
                            self.send(Command::DeckHotCue {
                                deck: d as u8,
                                pad: i as u8,
                                del: ui.input(|i| i.modifiers.shift),
                            });
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                for (band, lab) in [(0u8, "b"), (1, "m"), (2, "t"), (3, "g")] {
                    let cut = snap.eq_cut.get(band as usize).copied().unwrap_or(false);
                    let solo = snap.eq_solo == band as i8;
                    let v = if band < 3 {
                        eq_to_knob(snap.eq[band as usize])
                    } else {
                        (snap.gain / 1.2).clamp(0.0, 1.0)
                    };
                    let c = if cut { t.red } else if solo { t.yellow } else { col };
                    let resp = rotary(ui, t, lab, v, c, cell + 4.0);
                    if resp.changed {
                        if band < 3 {
                            self.send(Command::DeckEq { deck: d as u8, band, value: resp.value });
                        } else {
                            self.send(Command::DeckGain { deck: d as u8, value: resp.value * 1.2 });
                        }
                    }
                    if resp.clicked {
                        self.send(Command::DeckEqCut { deck: d as u8, band });
                    }
                    if resp.secondary {
                        self.send(Command::DeckEqSolo { deck: d as u8, band });
                    }
                }
            });
        });
    }

    fn platter_col(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, col: Color32, wave_h: f32) {
        ui.vertical(|ui| {
            ui.set_width(wave_h);
            let hit = platter(ui, t, snap, col, wave_h, |delta, touch| {
                self.send(Command::DeckTouch { deck: d as u8, on: touch });
                self.send(Command::DeckJog { deck: d as u8, delta });
            });
            if hit.shift_click {
                self.send(Command::DeckUnload { deck: d as u8 });
            } else if hit.right_click {
                self.send(Command::DeckCue { deck: d as u8 });
            } else if hit.click {
                self.send(Command::DeckPlay { deck: d as u8 });
            }
        });
    }

    fn btn_stack(&mut self, ui: &mut Ui, t: &Theme, d: usize, snap: &crate::engine::DeckSnap, sq: f32) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            ui.set_width(sq);
            let q = self.snap.quantize;
            if sq_btn(ui, t, "Q", q, t.yellow, sq).on_hover_text("quantize").clicked() {
                self.send(Command::ToggleQuant);
            }
            let io = sq_btn(ui, t, "I/O", snap.loop_on, t.accent, sq).on_hover_text("loop in · right-click out");
            if io.clicked() {
                self.send(Command::DeckLoopIn { deck: d as u8 });
            }
            if io.secondary_clicked() {
                self.send(Command::DeckLoopOut { deck: d as u8 });
            }
            if sq_btn(ui, t, "×2", false, t.cyan, sq).on_hover_text("double loop").clicked() {
                self.send(Command::DeckLoopDouble { deck: d as u8 });
            }
            if sq_btn(ui, t, "½", false, t.cyan, sq).on_hover_text("halve loop").clicked() {
                self.send(Command::DeckLoopHalf { deck: d as u8 });
            }
            if sq_btn(ui, t, "↻", snap.loop_on, t.magenta, sq).on_hover_text("reloop 4 bars").clicked() {
                self.send(Command::DeckReloop { deck: d as u8 });
            }
            if sq_btn(ui, t, "⇄", snap.sync, t.green, sq).on_hover_text("match").clicked() {
                self.send(Command::DeckMatch);
            }
        });
    }

    fn sampler_row(&mut self, ui: &mut Ui, t: &Theme) {
        let h = ui.available_height();
        let w = ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
            ui.vertical(|ui| {
                ui.set_width(108.0);
                ui.set_min_height(h);
                egui::ComboBox::from_id_salt("bank")
                    .selected_text(
                        self.snap
                            .sampler_banks
                            .get(self.snap.sampler_bank)
                            .cloned()
                            .unwrap_or_else(|| "bank".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, n) in self.snap.sampler_banks.iter().enumerate() {
                            if ui.selectable_label(i == self.snap.sampler_bank, n).clicked() {
                                self.send(Command::SamplerBank(i));
                            }
                        }
                    });
                let inst = match self.snap.sampler_inst {
                    -1 => "samples",
                    0 => "drums",
                    1 => "analog",
                    2 => "keys",
                    _ => "pad",
                };
                egui::ComboBox::from_id_salt("inst")
                    .selected_text(inst)
                    .show_ui(ui, |ui| {
                        for (i, n) in [(-1i8, "samples"), (0, "drums"), (1, "analog"), (2, "keys"), (3, "pad")] {
                            if ui.selectable_label(self.snap.sampler_inst == i, n).clicked() {
                                self.send(Command::SamplerInst(i));
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if sq_btn(ui, t, "^", false, t.accent, 26.0).clicked() {
                        self.send(Command::SamplerOct(1));
                    }
                    ui.label(RichText::new(format!("C{}", self.snap.sampler_oct)).size(11.0).color(t.fg));
                    if sq_btn(ui, t, "v", false, t.accent, 26.0).clicked() {
                        self.send(Command::SamplerOct(-1));
                    }
                });
            });
            let pad_w = ((w - 120.0 - 8.0) / 8.0).max(36.0);
            let pad_h = ((h - 4.0) / 2.0).max(32.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                let piano = self.snap.sampler_inst >= 0;
                let labels_top = if piano {
                    ["A#", "", "C#", "D#", "", "F#", "G#", ""]
                } else {
                    ["1", "2", "3", "4", "5", "6", "7", "8"]
                };
                let labels_bot = if piano {
                    ["A", "B", "C", "D", "E", "F", "G", "A"]
                } else {
                    ["9", "10", "11", "12", "13", "14", "15", "16"]
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                    for (c, lab) in labels_top.iter().enumerate() {
                        let p = 8 + c as u8;
                        let empty = lab.is_empty();
                        let r = pad_btn(ui, t, lab, empty, t.track_color(c), Vec2::new(pad_w, pad_h));
                        self.pad_gate(p as usize, !empty, &r);
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                    for (c, lab) in labels_bot.iter().enumerate() {
                        let p = c as u8;
                        let r = pad_btn(ui, t, lab, false, t.track_color(c + 8), Vec2::new(pad_w, pad_h));
                        self.pad_gate(p as usize, true, &r);
                    }
                });
            });
        });
    }

    fn crate_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("crate").size(11.0).color(t.fg_dim));
                ui.add(egui::TextEdit::singleline(&mut self.lib_filter).hint_text("search").desired_width(180.0));
                if ui.button("scan").clicked() {
                    self.scan_library();
                }
                if ui.button("→ A").clicked() {
                    self.load_sel(0);
                }
                if ui.button("→ B").clicked() {
                    self.load_sel(1);
                }
                ui.label(RichText::new("↓ bpm up   ↑ bpm down   same bpm → key → name").size(10.0).color(t.muted));
            });
            let header = ["song", "bpm", "key", "length", "last play", "artist"];
            let col_w = [280.0, 56.0, 48.0, 64.0, 140.0, 180.0];
            ui.horizontal(|ui| {
                for (h, w) in header.iter().zip(col_w.iter()) {
                    ui.add_sized(Vec2::new(*w, 16.0), egui::Label::new(RichText::new(*h).size(10.0).color(t.fg_dim)));
                }
            });
            let rows: Vec<(usize, String, String, String, String, String, String)> = self
                .filtered()
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    (
                        i,
                        it.title.clone(),
                        if it.bpm > 1.0 { format!("{:.1}", it.bpm) } else { "—".into() },
                        it.key.clone(),
                        fmt_len(it.length),
                        fmt_play(it.last_play),
                        it.artist.clone(),
                    )
                })
                .collect();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (i, title, bpm, key, len, last, artist) in &rows {
                    let sel = *i == self.lib_sel;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::click());
                    if sel {
                        ui.painter().rect_filled(rect, 2.0, t.accent.gamma_multiply(0.18));
                    }
                    let mut x = rect.left();
                    for (txt, w) in [title, bpm, key, len, last, artist].iter().zip(col_w) {
                        ui.painter().text(
                            Pos2::new(x + 4.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            *txt,
                            FontId::proportional(11.0),
                            if sel { t.accent } else { t.fg },
                        );
                        x += w;
                    }
                    if resp.clicked() {
                        self.lib_sel = *i;
                    }
                    if resp.double_clicked() {
                        self.lib_sel = *i;
                        self.load_sel(self.snap.selected_deck as u8);
                    }
                }
            });
        });
    }

    fn sequencer_row(&mut self, ui: &mut Ui, t: &Theme) {
        let avail = ui.available_size();
        let gap = 4.0;
        let head_h = 26.0;
        let gain_h = 48.0;
        let scene_w = 28.0;
        let cols = TRACKS as f32;
        let rows = SCENES as f32;
        let col_w = ((avail.x - scene_w - gap * (cols + 1.0)) / cols).max(36.0);
        let row_h = (t.font_size + 10.0).clamp(20.0, 26.0);
        let pack_w = scene_w + gap + cols * col_w + (cols - 1.0) * gap;
        let pack_h = head_h + gap + rows * row_h + (rows - 1.0) * gap + gap + gain_h;
        ui.spacing_mut().item_spacing = Vec2::splat(gap);
        ui.with_layout(egui::Layout::left_to_right(Align::Min), |ui| {
            ui.add_space(((avail.x - pack_w) * 0.5).max(0.0));
            ui.allocate_ui(Vec2::new(pack_w, pack_h.min(avail.y)), |ui| {
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(gap);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, head_h), Sense::hover());
                        for tr in 0..TRACKS {
                            let name = self.snap.tracks.get(tr).map(|x| x.name.as_str()).unwrap_or("tr");
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, head_h), Sense::click());
                            let fill = if mute {
                                t.red.gamma_multiply(0.35)
                            } else if solo {
                                t.yellow.gamma_multiply(0.35)
                            } else {
                                t.bg_dark
                            };
                            ui.painter().rect_filled(rect, 4.0, fill);
                            ui.painter().rect_stroke(rect, 4.0, st(1.0, t.track_color(tr).gamma_multiply(0.7)), egui::StrokeKind::Inside);
                            let fs = (col_w * 0.12).clamp(10.0, 13.0);
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, name, FontId::proportional(fs), t.track_color(tr));
                            if resp.clicked() {
                                self.send(Command::Mute { track: tr as u8 });
                            }
                            if resp.secondary_clicked() {
                                self.send(Command::Solo { track: tr as u8 });
                            }
                        }
                    });
                    for sc in 0..SCENES {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(gap);
                            let on = self.snap.tracks.iter().any(|tr| tr.playing_scene == sc as i8);
                            let (hr, hresp) = ui.allocate_exact_size(Vec2::new(scene_w, row_h), Sense::click());
                            ui.painter().rect_filled(hr, 4.0, if on { t.accent.gamma_multiply(0.45) } else { t.bg_dark });
                            ui.painter().rect_stroke(hr, 4.0, st(1.0, if on { t.accent } else { t.muted.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
                            ui.painter().text(hr.center(), egui::Align2::CENTER_CENTER, &format!("{}", sc + 1), FontId::proportional(11.0), t.fg);
                            if hresp.clicked() {
                                if ui.input(|i| i.modifiers.shift) {
                                    self.send(Command::AddScene { scene: sc as u8 });
                                } else if ui.input(|i| i.modifiers.ctrl) {
                                    self.send(Command::OpenFxScene(sc as u8));
                                } else {
                                    self.send(Command::ToggleScene { scene: sc as u8 });
                                }
                            }
                            if hresp.secondary_clicked() {
                                self.send(Command::RestartScene { scene: sc as u8 });
                            }
                            for tr in 0..TRACKS {
                                let clip = self.snap.tracks.get(tr).and_then(|x| x.clips.get(sc));
                                let filled = clip.map(|c| c.kind != 0).unwrap_or(false);
                                let playing = self.snap.tracks.get(tr).map(|x| x.playing_scene == sc as i8).unwrap_or(false);
                                let looping = self.snap.tracks.get(tr).map(|x| x.clip_looping).unwrap_or(false);
                                let color = t.track_color(tr);
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, row_h), Sense::click());
                                let fill = if playing {
                                    color.gamma_multiply(0.55)
                                } else if filled {
                                    color.gamma_multiply(0.22)
                                } else {
                                    t.bg_dark
                                };
                                ui.painter().rect_filled(rect, 4.0, fill);
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    st(if looping && playing { 2.0 } else { 1.0 }, color.gamma_multiply(0.65)),
                                    egui::StrokeKind::Inside,
                                );
                                if filled {
                                    let fs = (row_h * 0.38).clamp(10.0, 13.0);
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        clip.map(|c| c.name.as_str()).unwrap_or(""),
                                        FontId::proportional(fs),
                                        t.fg,
                                    );
                                    if playing {
                                        let w = rect.width() * self.snap.tracks.get(tr).map(|x| x.clip_progress).unwrap_or(0.0);
                                        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 3.0)), 0.0, t.fg);
                                    }
                                }
                                if resp.clicked() {
                                    if ui.input(|i| i.modifiers.shift) {
                                        self.send(Command::Select { track: tr, scene: sc });
                                        self.status = format!(
                                            "compose {} / scene {}",
                                            self.snap.tracks.get(tr).map(|x| x.name.as_str()).unwrap_or("?"),
                                            sc + 1
                                        );
                                    } else {
                                        self.send(Command::FireClip { track: tr as u8, scene: sc as u8, looping: false });
                                    }
                                }
                                if resp.secondary_clicked() {
                                    self.send(Command::FireClip { track: tr as u8, scene: sc as u8, looping: true });
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        let _ = ui.allocate_exact_size(Vec2::new(scene_w, gain_h), Sense::hover());
                        for tr in 0..TRACKS {
                            let g = self.snap.tracks.get(tr).map(|x| x.gain).unwrap_or(0.8);
                            let mute = self.snap.tracks.get(tr).map(|x| x.mute).unwrap_or(false);
                            let solo = self.snap.tracks.get(tr).map(|x| x.solo).unwrap_or(false);
                            let col = t.track_color(tr);
                            let c = if mute { t.red } else if solo { t.yellow } else { col };
                            let (cell, _) = ui.allocate_exact_size(Vec2::new(col_w, gain_h), Sense::hover());
                            let knob = (col_w.min(gain_h) - 2.0).clamp(28.0, 44.0);
                            let resp = rotary_in(ui, t, "gain", (g / 1.2).clamp(0.0, 1.0), c, knob, cell, tr);
                            if resp.changed {
                                self.send(Command::TrackGain { track: tr as u8, value: resp.value * 1.2 });
                            }
                            if resp.clicked {
                                if ui.input(|i| i.modifiers.ctrl) {
                                    self.send(Command::OpenFxTrack(tr as u8));
                                } else {
                                    self.send(Command::Mute { track: tr as u8 });
                                }
                            }
                            if resp.secondary {
                                self.send(Command::Solo { track: tr as u8 });
                            }
                        }
                    });
                });
            });
        });
    }

    fn fx_row(&mut self, ui: &mut Ui, t: &Theme) {
        ui.horizontal(|ui| {
            let label = if self.snap.fx_view >= 100 {
                format!("scene {} fx", self.snap.fx_view - 99)
            } else {
                format!(
                    "{} fx",
                    self.snap.tracks.get(self.snap.fx_view as usize).map(|x| x.name.as_str()).unwrap_or("track")
                )
            };
            ui.label(RichText::new(label).color(t.accent).strong());
            if pill(ui, t, "back", false, t.fg).clicked() {
                self.send(Command::CloseFx);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (i, id) in FxId::all().iter().enumerate() {
                if pill(ui, t, id.name(), false, t.cyan).clicked() {
                    self.send(Command::FxAdd(i as u8));
                }
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (name, on, mix, p)) in self.snap.fx_slots.clone().into_iter().enumerate() {
                ui.horizontal(|ui| {
                    if pill(ui, t, &name, on, t.green).clicked() {
                        self.send(Command::FxToggle(i));
                    }
                    let mut m = mix;
                    if ui.add(egui::Slider::new(&mut m, 0.0..=1.0).text("mix")).changed() {
                        self.send(Command::FxMix { slot: i, value: m });
                    }
                    for pi in 0..3 {
                        let mut v = p[pi];
                        if ui.add(egui::Slider::new(&mut v, 0.0..=1.0)).changed() {
                            self.send(Command::FxParam { slot: i, p: pi as u8, value: v });
                        }
                    }
                });
            }
            if self.snap.fx_slots.is_empty() {
                ui.label(RichText::new("add a device — chain runs top to bottom").color(t.muted));
            }
        });
    }
}

fn split_artist_title(stem: &str) -> (String, String) {
    if let Some((a, b)) = stem.split_once(" - ") {
        (a.trim().into(), b.trim().into())
    } else {
        (String::new(), stem.to_string())
    }
}

fn parse_tags(stem: &str) -> (f32, String) {
    let mut bpm = 0.0f32;
    let mut key = "—".into();
    for tok in stem.split(|c: char| !c.is_ascii_alphanumeric()) {
        if let Ok(n) = tok.parse::<f32>() {
            if (60.0..200.0).contains(&n) {
                bpm = n;
            }
        }
        let u = tok.to_uppercase();
        if matches!(u.as_str(), "A" | "B" | "C" | "D" | "E" | "F" | "G" | "AM" | "BM" | "CM" | "DM" | "EM" | "FM" | "GM" | "A#" | "C#" | "D#" | "F#" | "G#" | "BB" | "DB" | "EB" | "GB" | "AB") {
            if tok.len() <= 3 {
                key = u;
            }
        }
    }
    (bpm, key)
}

fn sort_crate(items: &mut [LibItem]) {
    items.sort_by(|a, b| {
        let ba = if a.bpm > 1.0 { a.bpm } else { 999.0 };
        let bb = if b.bpm > 1.0 { b.bpm } else { 999.0 };
        ba.partial_cmp(&bb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

fn fmt_len(s: f32) -> String {
    if s <= 0.0 {
        "—".into()
    } else {
        format!("{}:{:02}", (s as u32) / 60, (s as u32) % 60)
    }
}

fn fmt_play(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "—".into() };
    let Ok(d) = t.duration_since(SystemTime::UNIX_EPOCH) else { return "—".into() };
    format!("{}", d.as_secs() % 100000)
}

fn scratch_metrics(w: f32, h: f32, fader_h: f32, gap: f32) -> (f32, f32, f32, f32) {
    let mut wave_h = (h - fader_h - gap).max(120.0);
    let mut sq = 26.0;
    let mut side_w = 0.0;
    let mut mid_w = 0.0;
    for _ in 0..12 {
        sq = (wave_h / 7.0).clamp(22.0, 34.0);
        let cue_w = 4.0 * sq + 9.0;
        side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
        mid_w = w - 2.0 * side_w - 2.0 * gap;
        if mid_w >= 120.0 {
            break;
        }
        wave_h *= 0.92;
        if wave_h < 120.0 {
            wave_h = 120.0;
            sq = (wave_h / 7.0).clamp(22.0, 34.0);
            let cue_w = 4.0 * sq + 9.0;
            side_w = sq + gap + cue_w + gap + wave_h + gap + sq;
            mid_w = (w - 2.0 * side_w - 2.0 * gap).max(96.0);
            break;
        }
    }
    (sq, wave_h, side_w, mid_w.max(96.0))
}

fn st(width: f32, color: Color32) -> Stroke {
    Stroke { width, color }
}

fn pill(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32) -> egui::Response {
    let fill = if on { accent.gamma_multiply(0.35) } else { t.bg_light };
    let stroke = if on { accent } else { t.muted.gamma_multiply(0.5) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.0), if on { t.fg_bright } else { t.fg });
    let size = Vec2::new((galley.size().x + 12.0).max(28.0), 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().galley(Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5), galley, t.fg);
    resp
}

fn sq_btn(ui: &mut Ui, t: &Theme, text: &str, on: bool, accent: Color32, size: f32) -> egui::Response {
    let fill = if on { accent.gamma_multiply(0.4) } else { t.bg_light };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, st(1.0, if on { accent } else { t.muted.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(11.0), t.fg);
    resp
}

fn pad_btn(ui: &mut Ui, t: &Theme, text: &str, empty: bool, col: Color32, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    ui.painter().rect_filled(rect, 4.0, if empty { t.bg_darker } else { col.gamma_multiply(0.28) });
    ui.painter().rect_stroke(rect, 4.0, st(1.0, col.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    if !empty {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(12.0), t.fg);
    }
    resp
}

struct RotaryResp {
    value: f32,
    changed: bool,
    clicked: bool,
    secondary: bool,
}

fn rotary_in(
    ui: &mut Ui,
    t: &Theme,
    label: &str,
    value: f32,
    col: Color32,
    size: f32,
    cell: Rect,
    id: usize,
) -> RotaryResp {
    let cx = cell.center().x;
    let cy = cell.center().y - 6.0;
    let knob = Rect::from_center_size(Pos2::new(cx, cy), Vec2::splat(size));
    let resp = ui.interact(knob, ui.id().with(("rotary", id)), Sense::click_and_drag());
    let c = knob.center();
    let r = size * 0.38;
    ui.painter().circle_filled(c, r, t.bg_darker);
    ui.painter().circle_stroke(c, r, st(1.5, col));
    let ang = -2.2 + value.clamp(0.0, 1.0) * 4.4;
    let dir = Vec2::angled(ang);
    ui.painter().line_segment([c, c + dir * (r - 3.0)], st(2.0, col));
    ui.painter().text(
        Pos2::new(cx, knob.bottom() + 1.0),
        egui::Align2::CENTER_TOP,
        label,
        FontId::proportional(9.0),
        t.fg_dim,
    );
    let mut out = RotaryResp {
        value,
        changed: false,
        clicked: resp.clicked(),
        secondary: resp.secondary_clicked(),
    };
    if resp.dragged() {
        out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
        out.changed = true;
    }
    out
}

fn rotary(ui: &mut Ui, t: &Theme, label: &str, value: f32, col: Color32, size: f32) -> RotaryResp {
    ui.vertical(|ui| {
        ui.set_width(size);
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
        let c = rect.center();
        let r = size * 0.38;
        ui.painter().circle_filled(c, r, t.bg_darker);
        ui.painter().circle_stroke(c, r, st(1.5, col));
        let ang = -2.2 + value.clamp(0.0, 1.0) * 4.4;
        let dir = Vec2::angled(ang);
        ui.painter().line_segment([c, c + dir * (r - 3.0)], st(2.0, col));
        ui.label(RichText::new(label).size(9.0).color(t.fg_dim));
        let mut out = RotaryResp {
            value,
            changed: false,
            clicked: resp.clicked(),
            secondary: resp.secondary_clicked(),
        };
        if resp.dragged() {
            out.value = (value - resp.drag_delta().y * 0.01).clamp(0.0, 1.0);
            out.changed = true;
        }
        out
    })
    .inner
}

struct PlatterHit {
    click: bool,
    right_click: bool,
    shift_click: bool,
}

fn platter(
    ui: &mut Ui,
    t: &Theme,
    snap: &crate::engine::DeckSnap,
    col: Color32,
    size: f32,
    mut on_jog: impl FnMut(f32, bool),
) -> PlatterHit {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    let c = rect.center();
    let r = size * 0.47;
    let p = ui.painter();
    p.circle_filled(c, r, t.bg_darker);
    p.circle_stroke(c, r, st(2.0, col.gamma_multiply(0.85)));
    for i in 6..16 {
        p.circle_stroke(c, r * i as f32 / 18.0, st(0.5, t.muted.gamma_multiply(0.35)));
    }
    p.circle_filled(c, r * 0.38, col.gamma_multiply(0.28));
    let bpm = platter_bpm(snap);
    let remain = platter_remain(snap);
    p.text(
        c + Vec2::new(0.0, -8.0),
        egui::Align2::CENTER_CENTER,
        format!("{bpm:.1}"),
        FontId::proportional((size * 0.11).clamp(12.0, 18.0)),
        t.fg_bright,
    );
    p.text(
        c + Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        remain,
        FontId::monospace((size * 0.08).clamp(10.0, 14.0)),
        t.accent,
    );
    let angle = if snap.frames > 1.0 {
        (snap.pos / snap.frames) as f32 * std::f32::consts::TAU * 18.0
    } else {
        0.0
    };
    let dir = Vec2::angled(angle);
    p.line_segment([c + dir * 10.0, c + dir * (r - 4.0)], st(2.0, t.accent));
    if snap.playing {
        p.circle_filled(c + dir * (r - 7.0), 3.0, t.green);
    }
    let dragging = resp.dragged_by(PointerButton::Primary) && resp.drag_delta().length() > 1.5;
    if dragging {
        if let Some(pos) = resp.interact_pointer_pos() {
            let v = pos - c;
            if v.length() > 4.0 {
                let tangent = Vec2::new(-v.y, v.x).normalized();
                on_jog(resp.drag_delta().dot(tangent) / r, true);
            }
        }
    } else if resp.drag_stopped() {
        on_jog(0.0, false);
    }
    let shift = ui.input(|i| i.modifiers.shift);
    PlatterHit {
        click: resp.clicked() && !shift,
        right_click: resp.secondary_clicked(),
        shift_click: resp.clicked() && shift,
    }
}

fn vertical_wave(
    ui: &mut Ui,
    t: &Theme,
    snap: &crate::engine::DeckSnap,
    col: Color32,
    w: f32,
    h: f32,
    mut on_seek: impl FnMut(f32),
) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, t.bg_darker);
    if snap.peaks.is_empty() || snap.frames < 1.0 || snap.duration <= 0.01 {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "wave", FontId::proportional(10.0), t.muted);
        return;
    }
    let pos_s = (snap.pos / snap.frames) as f32 * snap.duration;
    let half = 3.5;
    let start_s = (pos_s - half).max(0.0);
    let end_s = (start_s + half * 2.0).min(snap.duration);
    let n = snap.peaks.len() as f32;
    let a = ((start_s / snap.duration) * n) as usize;
    let b = (((end_s / snap.duration) * n) as usize).max(a + 1).min(snap.peaks.len());
    let mid = rect.center().x;
    let hw = rect.width() * 0.46;
    let span = (b - a).max(1);
    let step = (span as f32 / rect.height().max(1.0)).ceil().max(1.0) as usize;
    let mut k = 0usize;
    let mut i = a;
    while i < b {
        let y = rect.top() + k as f32 * step as f32 / span as f32 * rect.height();
        let pk = snap.peaks[i];
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid - pk[0] * hw, y)], st(1.0, t.red.gamma_multiply(0.9)));
        p.line_segment([Pos2::new(mid, y), Pos2::new(mid + pk[1] * hw, y)], st(1.0, t.green.gamma_multiply(0.85)));
        p.line_segment([Pos2::new(mid - pk[2] * hw * 0.35, y), Pos2::new(mid + pk[2] * hw * 0.35, y)], st(1.0, col.gamma_multiply(0.5)));
        i += step;
        k += 1;
    }
    let play_y = rect.top() + ((pos_s - start_s) / (end_s - start_s).max(0.001)) * rect.height();
    p.line_segment([Pos2::new(rect.left(), play_y), Pos2::new(rect.right(), play_y)], st(1.6, t.accent));
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let u = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let tsec = start_s + u * (end_s - start_s);
            on_seek((tsec / snap.duration).clamp(0.0, 1.0));
        }
    }
}

fn fader(ui: &mut Ui, t: &Theme, value: f32, max: f32, meter: f32, col: Color32, width: f32, height: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let p = ui.painter();
    let track = Rect::from_center_size(rect.center(), Vec2::new(7.0, rect.height() - 8.0));
    p.rect_filled(track, 3.0, t.bg_darker);
    let mh = track.height() * meter.clamp(0.0, 1.0);
    p.rect_filled(Rect::from_min_max(Pos2::new(track.right() + 2.0, track.bottom() - mh), Pos2::new(track.right() + 5.0, track.bottom())), 1.0, t.green);
    let y = track.bottom() - (value / max.max(0.001)).clamp(0.0, 1.0) * track.height();
    p.rect_filled(Rect::from_center_size(Pos2::new(rect.center().x, y), Vec2::new(16.0, 7.0)), 2.0, col);
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            return Some((1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0) * max);
        }
    }
    None
}

fn xfader(ui: &mut Ui, t: &Theme, width: f32, height: f32, value: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, t.bg_darker);
    p.text(rect.left_center() + Vec2::new(6.0, 0.0), egui::Align2::LEFT_CENTER, "A", FontId::proportional(10.0), t.track_color(0));
    p.text(rect.right_center() - Vec2::new(6.0, 0.0), egui::Align2::RIGHT_CENTER, "B", FontId::proportional(10.0), t.track_color(1));
    let x = rect.left() + 16.0 + value.clamp(0.0, 1.0) * (rect.width() - 32.0);
    p.rect_filled(Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::new(12.0, 14.0)), 2.0, t.accent);
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            *value = ((pos.x - rect.left() - 16.0) / (rect.width() - 32.0)).clamp(0.0, 1.0);
            return true;
        }
    }
    false
}

fn platter_bpm(snap: &crate::engine::DeckSnap) -> f32 {
    let span = match snap.pitch_range {
        1 => 0.16,
        2 => 0.50,
        _ => 0.08,
    };
    let rate = 1.0 + (snap.pitch - 0.5) * 2.0 * span;
    snap.bpm.max(0.0) * rate
}

fn platter_remain(snap: &crate::engine::DeckSnap) -> String {
    if snap.frames < 1.0 || snap.duration <= 0.01 {
        return "—:——".into();
    }
    let left = snap.duration * (1.0 - (snap.pos / snap.frames) as f32).clamp(0.0, 1.0);
    let span = match snap.pitch_range {
        1 => 0.16,
        2 => 0.50,
        _ => 0.08,
    };
    let rate = (1.0 + (snap.pitch - 0.5) * 2.0 * span).max(0.05);
    let s = left / rate;
    let m = (s as u32) / 60;
    let sec = s % 60.0;
    format!("-{m}:{sec:04.1}")
}

fn eq_to_knob(g: f32) -> f32 {
    if g <= 1.0 {
        (g.max(0.0)).powf(1.0 / 1.4) * 0.5
    } else {
        0.5 + (g - 1.0) / 2.4 * 0.5
    }
}

const KEYS: &str = "\
SPACE session play/stop   F1 keys   CTRL+M midi
platter click = play   right-click = cue   shift-click = unload
platter shows playing BPM + time remaining
Q quant   I/O loop in / right-click out   ×2 ½ ↻ reloop ⇄ match
cues 1-8   bass/mid/treb/gain click=cut  rclick=solo
pitch L lock + 8/16/50 range
pads fill width · instrument mode A-G piano
crate: ↓ bpm up. load →A / →B
seq: scene head launch/stop, rclick restart, shift add
track head mute / rclick solo
cell click once / rclick loop / shift compose
gain rotary click mute / rclick solo / ctrl fx chain
";
