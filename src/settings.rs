//! The settings screen: an egui window over the terminal that edits the
//! main options of `config.toml` and saves them with [`config_edit`].

use std::path::PathBuf;

use egui::{Color32, ComboBox, DragValue, Grid, Key};

use crate::config::{Backdrop, Config, Fit, Shape};
use crate::{config_edit, theme};

pub struct Settings {
    path: Option<PathBuf>,
    /// The settings when the screen opened; only what differs is saved.
    before: Config,
    edited: Config,
    /// `font.family`, empty for the built-in font.
    family: String,
    profiles: Vec<String>,
    /// The result of the last save: the error, or `None` once saved.
    status: Option<Result<(), String>>,
}

impl Settings {
    pub fn new(config: &Config, path: Option<PathBuf>) -> Self {
        Self {
            path,
            before: config.clone(),
            edited: config.clone(),
            family: config.font.family.clone().unwrap_or_default(),
            profiles: config.profiles().into_iter().map(|p| p.name).collect(),
            status: None,
        }
    }

    /// Draws the screen; `false` once it is closed.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = !ctx.input(|i| i.key_pressed(Key::Escape));
        let mut close = false;
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
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        self.save();
                    }
                    close = ui.button("閉じる").clicked();
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
        }
        self.status = Some(result);
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
