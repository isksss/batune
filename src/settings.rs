//! The settings screen: an egui window over the terminal that edits the
//! main options of `config.toml` and saves them with [`config_edit`].

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use egui::{Color32, ComboBox, DragValue, Grid, Key};
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;
use crate::config::{Backdrop, Config, Fit, Shape};
use crate::update::{self, Outcome};
use crate::{config_edit, theme};

pub struct Settings {
    path: Option<PathBuf>,
    proxy: EventLoopProxy<UserEvent>,
    /// The settings when the screen opened; only what differs is saved.
    before: Config,
    edited: Config,
    /// `font.family`, empty for the built-in font.
    family: String,
    profiles: Vec<String>,
    /// The result of the last save: the error, or `None` once saved.
    status: Option<Result<(), String>>,
    /// The running update check; dropped with the screen, the check itself
    /// still finishes.
    updating: Option<Receiver<Result<Outcome, String>>>,
    update_status: Option<Result<String, String>>,
}

impl Settings {
    pub fn new(config: &Config, path: Option<PathBuf>, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            path,
            proxy,
            before: config.clone(),
            edited: config.clone(),
            family: config.font.family.clone().unwrap_or_default(),
            profiles: config.profiles().into_iter().map(|p| p.name).collect(),
            status: None,
            updating: None,
            update_status: None,
        }
    }

    /// Draws the screen; `false` once it is closed.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = !ctx.input(|i| i.key_pressed(Key::Escape));
        let mut close = false;
        if let Some(result) = self.updating.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.updating = None;
            self.update_status = Some(result.map(|outcome| match outcome {
                Outcome::UpToDate(v) => format!("最新版(v{v})です"),
                Outcome::Updated(v) => {
                    format!("v{v} に更新しました。batune を再起動すると反映されます")
                }
            }));
        }
        // Room left for the title, the message and the buttons, which stay
        // in view while the fields scroll.
        let fields_height = (ctx.content_rect().height() - 150.0).max(80.0);
        egui::Window::new("設定")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(fields_height)
                    .show(ui, |ui| {
                        Grid::new("settings")
                            .num_columns(2)
                            .striped(true)
                            .show(ui, |ui| self.fields(ui));
                    });
                ui.separator();
                match &self.status {
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xe0, 0x6c, 0x75), e);
                    }
                    Some(Ok(())) => {
                        ui.label("保存しました");
                    }
                    None => {}
                }
                match &self.update_status {
                    _ if self.updating.is_some() => {
                        ui.label("更新を確認しています…");
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xe0, 0x6c, 0x75), e);
                    }
                    Some(Ok(message)) => {
                        ui.label(message);
                    }
                    None => {}
                }
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        self.save();
                    }
                    close = ui.button("閉じる").clicked();
                    // Hidden rather than disabled for a Store install, which
                    // has nothing the user could do to enable it.
                    if update::available()
                        && ui
                            .add_enabled(self.updating.is_none(), egui::Button::new("更新を確認"))
                            .clicked()
                    {
                        self.check_update();
                    }
                });
            });
        open && !close
    }

    fn save(&mut self) {
        let family = self.family.trim();
        self.edited.font.family = (!family.is_empty()).then(|| family.to_owned());
        let result = match &self.path {
            Some(path) => config_edit::save(path, &self.before, &self.edited),
            None => Err("設定ファイルの場所が分かりません".to_owned()),
        };
        // The file now holds `edited`; a second save writes only what
        // changed after this one.
        if result.is_ok() {
            self.before = self.edited.clone();
            // Not left to the watch: there is none when the config directory
            // did not exist at startup. A second reload finds nothing changed.
            let _ = self.proxy.send_event(UserEvent::ConfigChanged);
        }
        self.status = Some(result);
    }

    /// Off the UI thread: the download takes seconds and can hang until the
    /// timeout.
    fn check_update(&mut self) {
        let (tx, rx) = mpsc::channel();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::run());
            let _ = proxy.send_event(UserEvent::Redraw);
        });
        self.updating = Some(rx);
        self.update_status = None;
    }

    fn fields(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.edited;
        row(ui, "既定のプロファイル", |ui| {
            ComboBox::from_id_salt("default_profile")
                .selected_text(c.default_profile.as_deref().unwrap_or("[shell]"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.default_profile, None, "[shell]");
                    for name in &self.profiles {
                        ui.selectable_value(&mut c.default_profile, Some(name.clone()), name);
                    }
                });
        });
        row(ui, "フォント", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.family).hint_text("内蔵(HackGen Console NF)"),
            );
        });
        // No `range` on the numbers: Config::parse checks them on save and
        // says what is wrong, instead of the value silently snapping back.
        row(ui, "文字の大きさ", |ui| {
            ui.add(DragValue::new(&mut c.font.size).speed(0.5));
        });
        row(ui, "行の高さ(倍)", |ui| {
            ui.add(DragValue::new(&mut c.font.line_height).speed(0.05));
        });
        row(ui, "セルの幅(倍)", |ui| {
            ui.add(DragValue::new(&mut c.font.cell_width).speed(0.05));
        });
        row(ui, "テーマ", |ui| {
            ComboBox::from_id_salt("theme")
                .selected_text(c.colors.theme.as_str())
                .show_ui(ui, |ui| {
                    for name in theme::names() {
                        ui.selectable_value(&mut c.colors.theme, name.to_owned(), name);
                    }
                });
        });
        row(ui, "", |ui| {
            ui.checkbox(&mut c.colors.bold_is_bright, "太字を明るい色で描く");
        });
        row(ui, "カーソルの形", |ui| {
            choice(
                ui,
                "shape",
                &mut c.cursor.shape,
                &[
                    (Shape::Block, "ブロック"),
                    (Shape::Underline, "下線"),
                    (Shape::Bar, "縦線"),
                ],
            );
        });
        row(ui, "", |ui| {
            ui.checkbox(&mut c.cursor.blinking, "カーソルを点滅させる");
        });
        row(ui, "点滅の間隔(ミリ秒)", |ui| {
            ui.add(DragValue::new(&mut c.cursor.blink_interval).speed(10));
        });
        row(ui, "点滅をやめるまで(秒)", |ui| {
            ui.add(DragValue::new(&mut c.cursor.blink_timeout));
        });
        row(ui, "スクロールバック(行)", |ui| {
            ui.add(DragValue::new(&mut c.scrolling.history).speed(100));
        });
        row(ui, "ホイール 1 目盛りの行数", |ui| {
            ui.add(DragValue::new(&mut c.scrolling.multiplier));
        });
        row(ui, "余白(ピクセル)", |ui| {
            ui.add(DragValue::new(&mut c.window.padding).speed(0.5));
        });
        row(ui, "タイトル", |ui| {
            ui.text_edit_singleline(&mut c.window.title);
        });
        row(ui, "", |ui| {
            ui.checkbox(
                &mut c.window.dynamic_title,
                "プログラムが付けたタイトルを使う",
            );
        });
        row(ui, "タイトルバーの背景", |ui| {
            choice(
                ui,
                "backdrop",
                &mut c.window.backdrop,
                &[
                    (Backdrop::None, "テーマの背景色"),
                    (Backdrop::Mica, "Mica(Windows 11)"),
                    (Backdrop::Acrylic, "Acrylic(Windows 11)"),
                ],
            );
        });
        row(ui, "背景画像の不透明度", |ui| {
            ui.add(DragValue::new(&mut c.background_image.opacity).speed(0.01));
        });
        row(ui, "背景画像の合わせ方", |ui| {
            choice(
                ui,
                "fit",
                &mut c.background_image.fit,
                &[
                    (Fit::Cover, "覆う"),
                    (Fit::Contain, "収める"),
                    (Fit::Stretch, "引き伸ばす"),
                ],
            );
        });
        row(ui, "", |ui| {
            ui.checkbox(&mut c.bell.visual, "ベルで画面を反転させる");
        });
        row(ui, "", |ui| {
            ui.checkbox(&mut c.selection.save_to_clipboard, "選択したらコピーする");
        });
        row(ui, "", |ui| {
            ui.checkbox(&mut c.clipboard.osc52_write, "OSC 52 での書き込みを許す");
        });
        row(ui, "", |ui| {
            ui.checkbox(
                &mut c.mouse.hide_when_typing,
                "入力中はマウスポインターを隠す",
            );
        });
    }
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(label);
    add(ui);
    ui.end_row();
}

fn choice<T: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, value: &mut T, options: &[(T, &str)]) {
    let current = options.iter().find(|(v, _)| v == value).map_or("", |o| o.1);
    ComboBox::from_id_salt(id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for &(v, label) in options {
                ui.selectable_value(value, v, label);
            }
        });
}
