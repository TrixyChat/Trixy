#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use eframe::egui;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use trixy::{
    default_db_path, firebase_probe, format_time, member_names, sync_once, AppDb, AttachmentView,
    MessageAlert, MessageView, SyncReport, ATTACHMENT_CHUNK_SIZE,
    MAX_ATTACHMENT_BYTES,
};

fn main() -> eframe::Result<()> {
    let db_path = env::var("TRIXY_DB")
        .or_else(|_| env::var("WORKMSG_DB"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_db_path());
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 780.0])
        .with_min_inner_size([900.0, 600.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/trixy-icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Trixy",
        native_options,
        Box::new(move |cc| {
            configure_light_theme(&cc.egui_ctx);
            Ok(Box::new(TrixyApp::new(db_path.clone())))
        }),
    )
}

fn configure_light_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = surface_color();
    visuals.window_fill = surface_color();
    visuals.window_stroke = egui::Stroke::new(1.0_f32, border_color());
    visuals.window_rounding = egui::Rounding::same(18.0);
    visuals.menu_rounding = egui::Rounding::same(12.0);
    visuals.extreme_bg_color = input_color();
    visuals.faint_bg_color = soft_color();
    visuals.code_bg_color = egui::Color32::from_rgb(232, 232, 237);
    visuals.error_fg_color = danger_color();
    visuals.selection.bg_fill = accent_color();
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);

    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, primary_color());
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border_color());
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(10.0);

    visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(250, 250, 252);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(250, 250, 252);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, border_color());
    visuals.widgets.inactive.rounding = egui::Rounding::same(10.0);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, primary_color());

    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(242, 242, 247);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(242, 242, 247);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(209, 209, 214));
    visuals.widgets.hovered.rounding = egui::Rounding::same(10.0);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, primary_color());

    visuals.widgets.active.weak_bg_fill = accent_soft_color();
    visuals.widgets.active.bg_fill = accent_soft_color();
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, accent_color());
    visuals.widgets.active.rounding = egui::Rounding::same(10.0);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, accent_color());

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(13.0, 8.0);
    style.spacing.interact_size.y = 36.0;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(25.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(15.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(14.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(12.5),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(14.0),
    );
    ctx.set_style(style);
}

// Apple-inspired light palette: airy surfaces, translucent-feeling grays, and one blue accent.
fn canvas_color() -> egui::Color32 { egui::Color32::from_rgb(244, 245, 248) }
fn sidebar_color() -> egui::Color32 { egui::Color32::from_rgb(246, 247, 250) }
fn surface_color() -> egui::Color32 { egui::Color32::from_rgb(255, 255, 255) }
fn soft_color() -> egui::Color32 { egui::Color32::from_rgb(242, 243, 247) }
fn input_color() -> egui::Color32 { egui::Color32::from_rgb(248, 249, 251) }
fn accent_color() -> egui::Color32 { egui::Color32::from_rgb(0, 122, 255) }
fn accent_soft_color() -> egui::Color32 { egui::Color32::from_rgb(232, 244, 255) }
fn border_color() -> egui::Color32 { egui::Color32::from_rgb(226, 228, 233) }
fn primary_color() -> egui::Color32 { egui::Color32::from_rgb(27, 28, 31) }
fn muted_color() -> egui::Color32 { egui::Color32::from_rgb(105, 107, 112) }
fn subtle_color() -> egui::Color32 { egui::Color32::from_rgb(146, 148, 154) }
fn success_color() -> egui::Color32 { egui::Color32::from_rgb(40, 194, 90) }
fn danger_color() -> egui::Color32 { egui::Color32::from_rgb(255, 59, 48) }

fn paint_logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, size * 0.28, accent_color());

    let bubble = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + size * 0.20, rect.min.y + size * 0.25),
        egui::pos2(rect.max.x - size * 0.20, rect.max.y - size * 0.27),
    );
    painter.rect_filled(bubble, size * 0.18, egui::Color32::WHITE);
    let dot_y = bubble.center().y;
    let r = size * 0.055;
    for offset in [-0.16_f32, 0.0, 0.16] {
        painter.circle_filled(
            egui::pos2(bubble.center().x + size * offset, dot_y),
            r,
            accent_color(),
        );
    }
}

fn avatar(ui: &mut egui::Ui, name: &str, size: f32, selected: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let fill = if selected { accent_color() } else { accent_soft_color() };
    let text = if selected { egui::Color32::WHITE } else { accent_color() };
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials(name),
        egui::FontId::proportional(size * 0.38),
        text,
    );
}

fn pill_frame(fill: egui::Color32) -> egui::Frame {
    egui::Frame::none()
        .fill(fill)
        .rounding(egui::Rounding::same(14.0))
        .inner_margin(8.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarMode {
    Workspaces,
    Contacts,
}

struct AlertToast {
    title: String,
    body: String,
    created_at: Instant,
}


struct TrixyApp {
    db: AppDb,
    selected_workspace: Option<String>,
    sidebar_mode: SidebarMode,
    sidebar_search: String,
    composer: String,
    pending_files: Vec<PathBuf>,
    setup_name: String,
    setup_firebase: String,
    show_identity: bool,
    show_add_person: bool,
    show_new_workspace: bool,
    show_people: bool,
    show_edit: bool,
    show_settings: bool,
    show_join_workspace: bool,
    show_share_workspace: bool,
    contact_code: String,
    workspace_contact_search: String,
    new_workspace_name: String,
    new_workspace_network_id: String,
    share_profile_network_id: String,
    join_workspace_code: String,
    workspace_share_link: String,
    new_network_label: String,
    new_network_url: String,
    edit_message_id: String,
    edit_message_text: String,
    status: String,
    status_is_error: bool,
    last_sync_start: Instant,
    syncing: bool,
    sync_rx: Option<Receiver<Result<SyncReport, String>>>,
    probe_rx: Option<Receiver<Result<(), String>>>,
    probing: bool,
    downloading_attachment: Option<String>,
    download_rx: Option<Receiver<Result<(String, PathBuf), String>>>,
    share_rx: Option<Receiver<Result<String, String>>>,
    sharing_workspace: bool,
    join_rx: Option<Receiver<Result<String, String>>>,
    joining_workspace: bool,
    alert_toast: Option<AlertToast>,
}

impl TrixyApp {
    fn new(db_path: PathBuf) -> Self {
        let db = AppDb::open(&db_path).unwrap_or_else(|err| {
            panic!(
                "Could not open Trixy database at {}: {err:#}",
                db_path.display()
            )
        });
        let selected_workspace = db
            .workspaces()
            .ok()
            .and_then(|items| items.first().map(|workspace| workspace.id.clone()));
        let initial_network = db.default_network_id().unwrap_or_default();
        Self {
            db,
            selected_workspace,
            sidebar_mode: SidebarMode::Workspaces,
            sidebar_search: String::new(),
            composer: String::new(),
            pending_files: Vec::new(),
            setup_name: String::new(),
            setup_firebase: "https://YOUR-PROJECT-default-rtdb.firebaseio.com".to_string(),
            show_identity: false,
            show_add_person: false,
            show_new_workspace: false,
            show_people: false,
            show_edit: false,
            show_settings: false,
            show_join_workspace: false,
            show_share_workspace: false,
            contact_code: String::new(),
            workspace_contact_search: String::new(),
            new_workspace_name: String::new(),
            new_workspace_network_id: initial_network.clone(),
            share_profile_network_id: initial_network,
            join_workspace_code: String::new(),
            workspace_share_link: String::new(),
            new_network_label: String::new(),
            new_network_url: String::new(),
            edit_message_id: String::new(),
            edit_message_text: String::new(),
            status: String::new(),
            status_is_error: false,
            last_sync_start: Instant::now() - Duration::from_secs(60),
            syncing: false,
            sync_rx: None,
            probe_rx: None,
            probing: false,
            downloading_attachment: None,
            download_rx: None,
            share_rx: None,
            sharing_workspace: false,
            join_rx: None,
            joining_workspace: false,
            alert_toast: None,
        }
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_is_error = error;
    }

    fn start_probe(&mut self) {
        if self.probing {
            return;
        }
        let url = self.setup_firebase.clone();
        let (tx, rx) = mpsc::channel();
        self.probing = true;
        self.probe_rx = Some(rx);
        std::thread::spawn(move || {
            let result = firebase_probe(&url).map_err(|err| format!("{err:#}"));
            let _ = tx.send(result);
        });
    }

    fn poll_probe(&mut self) {
        let Some(rx) = self.probe_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(())) => {
                self.probing = false;
                self.probe_rx = None;
                self.set_status("Connection test passed", false);
            }
            Ok(Err(err)) => {
                self.probing = false;
                self.probe_rx = None;
                self.set_status(format!("Connection test failed: {err}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.probing = false;
                self.probe_rx = None;
                self.set_status("Connection test stopped unexpectedly", true);
            }
        }
    }

    fn start_workspace_share(&mut self, workspace_id: &str) {
        if self.sharing_workspace {
            return;
        }
        let workspace_id = workspace_id.to_string();
        let db_path = self.db.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.sharing_workspace = true;
        self.workspace_share_link.clear();
        self.share_rx = Some(rx);
        std::thread::spawn(move || {
            let result = AppDb::open(db_path)
                .and_then(|db| db.create_workspace_share_link(&workspace_id))
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(result);
        });
    }

    fn poll_workspace_share(&mut self) {
        let Some(rx) = self.share_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(link)) => {
                self.sharing_workspace = false;
                self.share_rx = None;
                self.workspace_share_link = link;
                self.set_status("Workspace link ready", false);
            }
            Ok(Err(err)) => {
                self.sharing_workspace = false;
                self.share_rx = None;
                self.set_status(format!("Could not create workspace link: {err}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.sharing_workspace = false;
                self.share_rx = None;
                self.set_status("Workspace link creation stopped unexpectedly", true);
            }
        }
    }

    fn start_join_workspace(&mut self) {
        if self.joining_workspace || self.join_workspace_code.trim().is_empty() {
            return;
        }
        let code = self.join_workspace_code.clone();
        let db_path = self.db.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.joining_workspace = true;
        self.join_rx = Some(rx);
        std::thread::spawn(move || {
            let result = AppDb::open(db_path)
                .and_then(|db| db.import_workspace_share_link(&code))
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(result);
        });
    }

    fn poll_join_workspace(&mut self) {
        let Some(rx) = self.join_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(workspace_id)) => {
                self.joining_workspace = false;
                self.join_rx = None;
                self.selected_workspace = Some(workspace_id);
                self.sidebar_mode = SidebarMode::Workspaces;
                self.join_workspace_code.clear();
                self.show_join_workspace = false;
                self.set_status("Joined workspace · contacts imported", false);
                self.start_sync();
            }
            Ok(Err(err)) => {
                self.joining_workspace = false;
                self.join_rx = None;
                self.set_status(format!("Could not join workspace: {err}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.joining_workspace = false;
                self.join_rx = None;
                self.set_status("Workspace join stopped unexpectedly", true);
            }
        }
    }

    fn start_sync(&mut self) {
        if self.syncing || !self.db.has_identity().unwrap_or(false) {
            return;
        }
        let path = self.db.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.syncing = true;
        self.last_sync_start = Instant::now();
        self.sync_rx = Some(rx);
        std::thread::spawn(move || {
            let result = sync_once(path).map_err(|err| format!("{err:#}"));
            let _ = tx.send(result);
        });
    }

    fn poll_sync(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.sync_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(report)) => {
                self.syncing = false;
                self.sync_rx = None;

                if !report.alerts.is_empty() {
                    self.handle_message_alerts(ctx, &report.alerts);
                }

                if report.errors.is_empty() {
                    if report.sent > 0 || report.received > 0 {
                        self.set_status(
                            format!("Synced · {} sent · {} received", report.sent, report.received),
                            false,
                        );
                    }
                } else {
                    self.set_status(report.errors.join(" | "), true);
                }
                if self.selected_workspace.is_none() {
                    self.selected_workspace = self
                        .db
                        .workspaces()
                        .ok()
                        .and_then(|items| items.first().map(|workspace| workspace.id.clone()));
                }
            }
            Ok(Err(err)) => {
                self.syncing = false;
                self.sync_rx = None;
                self.set_status(format!("Sync unavailable: {err}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.syncing = false;
                self.sync_rx = None;
                self.set_status("Sync stopped unexpectedly", true);
            }
        }
    }

    fn handle_message_alerts(&mut self, ctx: &egui::Context, alerts: &[MessageAlert]) {
        let Some(last) = alerts.last() else { return; };
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));

        let title = if alerts.len() == 1 {
            format!("{} · {}", last.author_name, last.workspace_name)
        } else {
            format!("{} new messages", alerts.len())
        };
        let body = if last.body.trim().is_empty() && last.has_attachments {
            "Sent a file".to_string()
        } else {
            compact_preview(&last.body, 120)
        };
        self.alert_toast = Some(AlertToast {
            title: title.clone(),
            body: body.clone(),
            created_at: Instant::now(),
        });
        play_message_alert(&title, &body);
    }


    fn start_attachment_download(&mut self, attachment: &AttachmentView) {
        if self.downloading_attachment.is_some() {
            return;
        }
        let attachment_id = attachment.id.clone();
        let db_path = self.db.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.downloading_attachment = Some(attachment_id.clone());
        self.download_rx = Some(rx);
        std::thread::spawn(move || {
            let result = AppDb::open(db_path)
                .and_then(|db| db.download_attachment(&attachment_id))
                .map(|path| (attachment_id, path))
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send(result);
        });
    }

    fn poll_attachment_download(&mut self) {
        let Some(rx) = self.download_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((_id, path))) => {
                self.downloading_attachment = None;
                self.download_rx = None;
                self.set_status(format!("Downloaded {}", display_file_name(&path)), false);
            }
            Ok(Err(err)) => {
                self.downloading_attachment = None;
                self.download_rx = None;
                self.set_status(format!("Attachment download failed: {err}"), true);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.downloading_attachment = None;
                self.download_rx = None;
                self.set_status("Attachment download stopped unexpectedly", true);
            }
        }
    }

    fn setup_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(canvas_color()))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(54.0);
                    paint_logo(ui, 64.0);
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new("Trixy")
                            .size(36.0)
                            .strong()
                            .color(primary_color()),
                    );
                    ui.label(
                        egui::RichText::new("Private project messaging, without the clutter.")
                            .size(16.0)
                            .color(muted_color()),
                    );
                    ui.add_space(28.0);
                });

                ui.vertical_centered(|ui| {
                    egui::Frame::none()
                        .fill(surface_color())
                        .stroke(egui::Stroke::new(1.0_f32, border_color()))
                        .rounding(egui::Rounding::same(20.0))
                        .inner_margin(26.0)
                        .show(ui, |ui| {
                            ui.set_width(560.0);
                            ui.label(
                                egui::RichText::new("Set up this Mac or PC")
                                    .size(22.0)
                                    .strong()
                                    .color(primary_color()),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Your identity stays on this computer. Trixy uses Firebase only as an encrypted mailbox.",
                                )
                                .color(muted_color()),
                            );
                            ui.add_space(20.0);

                            ui.label(egui::RichText::new("Your name").strong());
                            ui.add_sized(
                                [ui.available_width(), 42.0],
                                egui::TextEdit::singleline(&mut self.setup_name)
                                    .hint_text("Your name"),
                            );
                            ui.add_space(10.0);

                            ui.label(egui::RichText::new("Firebase database URL").strong());
                            ui.add_sized(
                                [ui.available_width(), 42.0],
                                egui::TextEdit::singleline(&mut self.setup_firebase),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "This becomes your first Firebase connection. You can add more later in Settings.",
                                )
                                .small()
                                .color(muted_color()),
                            );
                            ui.add_space(20.0);

                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        !self.probing,
                                        egui::Button::new(if self.probing {
                                            "Testing…"
                                        } else {
                                            "Test connection"
                                        })
                                        .min_size(egui::vec2(124.0, 40.0)),
                                    )
                                    .clicked()
                                {
                                    self.start_probe();
                                }

                                let create = egui::Button::new(
                                    egui::RichText::new("Create profile")
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(accent_color())
                                .rounding(egui::Rounding::same(12.0))
                                .min_size(egui::vec2(136.0, 40.0));
                                if ui.add(create).clicked() {
                                    match self
                                        .db
                                        .create_identity(&self.setup_name, &self.setup_firebase)
                                    {
                                        Ok(_) => {
                                            self.set_status("Profile created. Trixy is ready.", false);
                                            self.start_sync();
                                        }
                                        Err(err) => self.set_status(format!("{err:#}"), true),
                                    }
                                }
                            });
                        });
                });
                self.status_line(ui);
            });
    }

    fn status_line(&self, ui: &mut egui::Ui) {
        if self.status.is_empty() {
            return;
        }
        ui.add_space(8.0);
        let fill = if self.status_is_error {
            egui::Color32::from_rgb(255, 238, 237)
        } else {
            soft_color()
        };
        let color = if self.status_is_error {
            danger_color()
        } else {
            muted_color()
        };
        egui::Frame::none()
            .fill(fill)
            .rounding(egui::Rounding::same(9.0))
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(&self.status).small().color(color));
            });
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(78.0)
            .frame(egui::Frame::none().fill(canvas_color()).inner_margin(10.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Brand capsule: deliberately separate from the actions so the
                    // toolbar feels like floating Mac chrome instead of a rigid strip.
                    egui::Frame::none()
                        .fill(surface_color())
                        .rounding(egui::Rounding::same(19.0))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                paint_logo(ui, 36.0);
                                ui.add_space(2.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("Trixy")
                                            .size(18.5)
                                            .strong()
                                            .color(primary_color()),
                                    );
                                    let who = self
                                        .db
                                        .identity()
                                        .map(|identity| identity.name)
                                        .unwrap_or_else(|_| "Profile".to_string());
                                    ui.label(
                                        egui::RichText::new(who)
                                            .size(11.0)
                                            .color(muted_color()),
                                    );
                                });
                            });
                        });

                    ui.add_space(7.0);
                    let network_count = self.db.networks().map(|items| items.len()).unwrap_or(0);
                    egui::Frame::none()
                        .fill(surface_color())
                        .rounding(egui::Rounding::same(18.0))
                        .inner_margin(9.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("●")
                                        .size(9.0)
                                        .color(if self.status_is_error {
                                            danger_color()
                                        } else {
                                            success_color()
                                        }),
                                );
                                ui.label(
                                    egui::RichText::new(if self.syncing {
                                        "Syncing…"
                                    } else if self.status_is_error {
                                        "Connection issue"
                                    } else {
                                        "Online"
                                    })
                                    .size(11.5)
                                    .color(muted_color()),
                                )
                                .on_hover_text(format!(
                                    "{} Firebase connection{} configured",
                                    network_count,
                                    if network_count == 1 { "" } else { "s" }
                                ));
                            });
                        });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::Frame::none()
                            .fill(surface_color())
                            .rounding(egui::Rounding::same(19.0))
                            .inner_margin(5.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new("Settings")
                                                .fill(soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(13.0)),
                                        )
                                        .clicked()
                                    {
                                        self.show_settings = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new("Share profile")
                                                .fill(soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(13.0)),
                                        )
                                        .clicked()
                                    {
                                        self.show_identity = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new("Add person")
                                                .fill(soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(13.0)),
                                        )
                                        .clicked()
                                    {
                                        self.show_add_person = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new("Join")
                                                .fill(accent_soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(13.0)),
                                        )
                                        .on_hover_text("Join a shared workspace")
                                        .clicked()
                                    {
                                        self.show_join_workspace = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(if self.syncing { "…" } else { "Sync" })
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(13.0)),
                                        )
                                        .on_hover_text("Sync now")
                                        .clicked()
                                    {
                                        self.start_sync();
                                    }
                                });
                            });
                    });
                });
            });
    }


    fn workspace_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("workspaces")
            .resizable(false)
            .default_width(292.0)
            .frame(egui::Frame::none().fill(canvas_color()).inner_margin(10.0))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(sidebar_color())
                    .rounding(egui::Rounding::same(24.0))
                    .inner_margin(13.0)
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());

                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(match self.sidebar_mode {
                                        SidebarMode::Workspaces => "Your spaces",
                                        SidebarMode::Contacts => "Your people",
                                    })
                                    .size(17.0)
                                    .strong()
                                    .color(primary_color()),
                                );
                                ui.label(
                                    egui::RichText::new(match self.sidebar_mode {
                                        SidebarMode::Workspaces => "Projects and conversations",
                                        SidebarMode::Contacts => "People you can work with",
                                    })
                                    .size(10.5)
                                    .color(muted_color()),
                                );
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_sized(
                                        [36.0, 36.0],
                                        egui::Button::new(
                                            egui::RichText::new("+")
                                                .size(18.0)
                                                .strong()
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(accent_color())
                                        .stroke(egui::Stroke::NONE)
                                        .rounding(egui::Rounding::same(18.0)),
                                    )
                                    .on_hover_text(match self.sidebar_mode {
                                        SidebarMode::Workspaces => "New workspace",
                                        SidebarMode::Contacts => "Add person",
                                    })
                                    .clicked()
                                {
                                    if self.sidebar_mode == SidebarMode::Workspaces {
                                        if self.new_workspace_network_id.is_empty() {
                                            self.new_workspace_network_id =
                                                self.db.default_network_id().unwrap_or_default();
                                        }
                                        self.show_new_workspace = true;
                                    } else {
                                        self.show_add_person = true;
                                    }
                                }
                            });
                        });
                        ui.add_space(12.0);

                        // macOS-style segmented control.
                        egui::Frame::none()
                            .fill(egui::Color32::from_rgb(235, 237, 242))
                            .rounding(egui::Rounding::same(15.0))
                            .inner_margin(3.0)
                            .show(ui, |ui| {
                                let width = ui.available_width();
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    let each = (width - 3.0) / 2.0;
                                    let workspace_selected =
                                        self.sidebar_mode == SidebarMode::Workspaces;
                                    if ui
                                        .add_sized(
                                            [each, 31.0],
                                            egui::Button::new("Workspaces")
                                                .fill(if workspace_selected {
                                                    surface_color()
                                                } else {
                                                    egui::Color32::TRANSPARENT
                                                })
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(12.0)),
                                        )
                                        .clicked()
                                    {
                                        self.sidebar_mode = SidebarMode::Workspaces;
                                        self.sidebar_search.clear();
                                    }
                                    let contacts_selected =
                                        self.sidebar_mode == SidebarMode::Contacts;
                                    if ui
                                        .add_sized(
                                            [each, 31.0],
                                            egui::Button::new("Contacts")
                                                .fill(if contacts_selected {
                                                    surface_color()
                                                } else {
                                                    egui::Color32::TRANSPARENT
                                                })
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(12.0)),
                                        )
                                        .clicked()
                                    {
                                        self.sidebar_mode = SidebarMode::Contacts;
                                        self.sidebar_search.clear();
                                    }
                                });
                            });
                        ui.add_space(10.0);

                        egui::Frame::none()
                            .fill(surface_color())
                            .rounding(egui::Rounding::same(14.0))
                            .inner_margin(7.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("⌕")
                                            .size(18.0)
                                            .color(subtle_color()),
                                    );
                                    let hint = if self.sidebar_mode == SidebarMode::Workspaces {
                                        "Search workspaces"
                                    } else {
                                        "Search contacts"
                                    };
                                    ui.add_sized(
                                        [ui.available_width(), 28.0],
                                        egui::TextEdit::singleline(&mut self.sidebar_search)
                                            .hint_text(hint)
                                            .frame(false),
                                    );
                                });
                            });
                        ui.add_space(10.0);

                        let list_height = (ui.available_height() - 72.0).max(140.0);
                        egui::ScrollArea::vertical()
                            .max_height(list_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let query = self.sidebar_search.trim().to_lowercase();
                                match self.sidebar_mode {
                                    SidebarMode::Workspaces => match self.db.workspaces() {
                                        Ok(workspaces) => {
                                            let mut shown = 0usize;
                                            for workspace in workspaces {
                                                if !query.is_empty()
                                                    && !workspace.name.to_lowercase().contains(&query)
                                                    && !workspace
                                                        .network_label
                                                        .to_lowercase()
                                                        .contains(&query)
                                                {
                                                    continue;
                                                }
                                                shown += 1;
                                                let selected = self.selected_workspace.as_deref()
                                                    == Some(workspace.id.as_str());
                                                let inner = egui::Frame::none()
                                                    .fill(if selected {
                                                        surface_color()
                                                    } else {
                                                        egui::Color32::TRANSPARENT
                                                    })
                                                    .stroke(if selected {
                                                        egui::Stroke::new(1.0_f32, border_color())
                                                    } else {
                                                        egui::Stroke::NONE
                                                    })
                                                    .rounding(egui::Rounding::same(15.0))
                                                    .inner_margin(9.0)
                                                    .show(ui, |ui| {
                                                        ui.horizontal(|ui| {
                                                            avatar(ui, &workspace.name, 34.0, selected);
                                                            ui.add_space(4.0);
                                                            ui.vertical(|ui| {
                                                                ui.label(
                                                                    egui::RichText::new(&workspace.name)
                                                                        .size(14.0)
                                                                        .strong()
                                                                        .color(primary_color()),
                                                                );
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        &workspace.network_label,
                                                                    )
                                                                    .size(10.5)
                                                                    .color(muted_color()),
                                                                );
                                                            });
                                                        });
                                                    });
                                                let response = ui.interact(
                                                    inner.response.rect,
                                                    ui.id().with(("workspace", &workspace.id)),
                                                    egui::Sense::click(),
                                                );
                                                if response.clicked() {
                                                    self.selected_workspace = Some(workspace.id);
                                                    self.pending_files.clear();
                                                }
                                                ui.add_space(4.0);
                                            }
                                            if shown == 0 {
                                                ui.add_space(20.0);
                                                ui.vertical_centered(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(if query.is_empty() {
                                                            "No workspaces yet"
                                                        } else {
                                                            "No matching workspaces"
                                                        })
                                                        .strong()
                                                        .color(muted_color()),
                                                    );
                                                });
                                            }
                                        }
                                        Err(err) => {
                                            ui.label(format!("Database error: {err}"));
                                        }
                                    },
                                    SidebarMode::Contacts => match self.db.contact_summaries() {
                                        Ok(contacts) => {
                                            let mut shown = 0usize;
                                            for contact in contacts {
                                                let route_text = contact
                                                    .routes
                                                    .iter()
                                                    .map(|route| route.label.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(" · ");
                                                if !query.is_empty()
                                                    && !contact.name.to_lowercase().contains(&query)
                                                    && !route_text.to_lowercase().contains(&query)
                                                {
                                                    continue;
                                                }
                                                shown += 1;
                                                egui::Frame::none()
                                                    .fill(egui::Color32::TRANSPARENT)
                                                    .rounding(egui::Rounding::same(15.0))
                                                    .inner_margin(9.0)
                                                    .show(ui, |ui| {
                                                        ui.horizontal(|ui| {
                                                            avatar(ui, &contact.name, 34.0, false);
                                                            ui.add_space(4.0);
                                                            ui.vertical(|ui| {
                                                                ui.label(
                                                                    egui::RichText::new(&contact.name)
                                                                        .size(14.0)
                                                                        .strong()
                                                                        .color(primary_color()),
                                                                );
                                                                ui.label(
                                                                    egui::RichText::new(
                                                                        if route_text.is_empty() {
                                                                            "No active database route"
                                                                        } else {
                                                                            route_text.as_str()
                                                                        },
                                                                    )
                                                                    .size(10.5)
                                                                    .color(muted_color()),
                                                                );
                                                            });
                                                        });
                                                    });
                                                ui.add_space(4.0);
                                            }
                                            if shown == 0 {
                                                ui.add_space(20.0);
                                                ui.vertical_centered(|ui| {
                                                    ui.label(
                                                        egui::RichText::new(if query.is_empty() {
                                                            "No contacts yet"
                                                        } else {
                                                            "No matching contacts"
                                                        })
                                                        .strong()
                                                        .color(muted_color()),
                                                    );
                                                });
                                            }
                                        }
                                        Err(err) => {
                                            ui.label(format!("Database error: {err}"));
                                        }
                                    },
                                }
                            });

                        ui.add_space(7.0);
                        let networks = self.db.networks().unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("●")
                                    .size(8.0)
                                    .color(if self.status_is_error {
                                        danger_color()
                                    } else {
                                        success_color()
                                    }),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} database{} · private · encrypted",
                                    networks.len(),
                                    if networks.len() == 1 { "" } else { "s" }
                                ))
                                .size(10.0)
                                .color(muted_color()),
                            );
                        });
                    });
            });
    }


    fn conversation_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(surface_color()).inner_margin(20.0))
            .show(ctx, |ui| {
                let Some(workspace_id) = self.selected_workspace.clone() else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(170.0);
                        ui.label(
                            egui::RichText::new("Choose a workspace")
                                .size(26.0)
                                .strong()
                                .color(primary_color()),
                        );
                        ui.label(
                            egui::RichText::new("Or create one to start a project conversation.")
                                .color(muted_color()),
                        );
                        ui.add_space(14.0);
                        if ui
                            .add(
                                egui::Button::new("Create workspace")
                                    .fill(accent_soft_color())
                                    .rounding(egui::Rounding::same(12.0)),
                            )
                            .clicked()
                        {
                            self.show_new_workspace = true;
                        }
                    });
                    return;
                };

                let name = self
                    .db
                    .workspace_name(&workspace_id)
                    .unwrap_or_else(|_| "Workspace".to_string());
                let members = self.db.members(&workspace_id).unwrap_or_default();
                let network = self.db.workspace_network(&workspace_id).ok();

                egui::Frame::none()
                    .fill(surface_color())
                    .inner_margin(0.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&name)
                                        .size(25.0)
                                        .strong()
                                        .color(primary_color()),
                                );
                                ui.horizontal(|ui| {
                                    if !members.is_empty() {
                                        ui.label(
                                            egui::RichText::new(member_names(&members))
                                                .small()
                                                .color(muted_color()),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("Just you for now")
                                                .small()
                                                .color(muted_color()),
                                        );
                                    }
                                    if let Some(network) = &network {
                                        ui.label(egui::RichText::new("·").small().color(subtle_color()));
                                        ui.label(
                                            egui::RichText::new(&network.label)
                                                .small()
                                                .color(accent_color()),
                                        );
                                    }
                                });
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(format!("People  {}", members.len()))
                                                .fill(soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(12.0)),
                                        )
                                        .clicked()
                                    {
                                        self.show_people = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new("Share workspace")
                                                .fill(accent_soft_color())
                                                .stroke(egui::Stroke::NONE)
                                                .rounding(egui::Rounding::same(12.0)),
                                        )
                                        .clicked()
                                    {
                                        self.workspace_share_link.clear();
                                        self.show_share_workspace = true;
                                    }
                                },
                            );
                        });
                    });
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add_space(14.0);
                        let messages = self.db.messages(&workspace_id).unwrap_or_default();
                        let my_id = self
                            .db
                            .identity()
                            .map(|identity| identity.user_id)
                            .unwrap_or_default();

                        if messages.is_empty() {
                            ui.add_space(70.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("No messages yet")
                                        .size(18.0)
                                        .strong()
                                        .color(primary_color()),
                                );
                                ui.label(
                                    egui::RichText::new("Start the conversation below.")
                                        .color(muted_color()),
                                );
                            });
                        }

                        ui.scope(|ui| {
                            ui.set_max_width(920.0);
                            for message in messages {
                                self.message_row(ui, &workspace_id, &my_id, message);
                                ui.add_space(10.0);
                            }
                        });
                        ui.add_space(14.0);
                    });
            });
    }

    fn composer_panel(&mut self, ctx: &egui::Context) {
        let Some(workspace_id) = self.selected_workspace.clone() else {
            return;
        };
        let height = if self.pending_files.is_empty() { 112.0 } else { 154.0 };
        egui::TopBottomPanel::bottom("composer_panel")
            .resizable(false)
            .exact_height(height)
            .frame(
                egui::Frame::none()
                    .fill(surface_color())
                    .stroke(egui::Stroke::new(1.0_f32, border_color()))
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                self.composer_ui(ui, &workspace_id);
            });
    }

    fn composer_ui(&mut self, ui: &mut egui::Ui, workspace_id: &str) {
        if !self.pending_files.is_empty() {
            let mut remove_index = None;
            egui::ScrollArea::horizontal()
                .auto_shrink([false, true])
                .max_height(34.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, path) in self.pending_files.iter().enumerate() {
                            let size = path
                                .metadata()
                                .map(|meta| format_file_size(meta.len()))
                                .unwrap_or_else(|_| "file".to_string());
                            egui::Frame::none()
                                .fill(soft_color())
                                .stroke(egui::Stroke::new(1.0_f32, border_color()))
                                .rounding(egui::Rounding::same(10.0))
                                .inner_margin(6.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(compact_preview(
                                                &display_file_name(path),
                                                34,
                                            ))
                                            .small()
                                            .strong()
                                            .color(primary_color()),
                                        );
                                        ui.label(
                                            egui::RichText::new(size)
                                                .small()
                                                .color(muted_color()),
                                        );
                                        if ui.small_button("×").clicked() {
                                            remove_index = Some(index);
                                        }
                                    });
                                });
                        }
                    });
                });
            if let Some(index) = remove_index {
                self.pending_files.remove(index);
            }
            ui.add_space(7.0);
        }

        egui::Frame::none()
            .fill(input_color())
            .stroke(egui::Stroke::new(1.0_f32, border_color()))
            .rounding(egui::Rounding::same(22.0))
            .inner_margin(7.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("+").size(20.0).strong())
                                .min_size(egui::vec2(38.0, 38.0))
                                .rounding(egui::Rounding::same(19.0)),
                        )
                        .on_hover_text("Attach file")
                        .clicked()
                    {
                        if let Some(files) = rfd::FileDialog::new().pick_files() {
                            self.add_pending_files(files);
                        }
                    }

                    let input_width = (ui.available_width() - 54.0).max(140.0);
                    let edit = egui::TextEdit::multiline(&mut self.composer)
                        .desired_rows(2)
                        .frame(false)
                        .hint_text("Message this workspace…");
                    ui.add_sized([input_width, 46.0], edit);

                    let can_send = !self.composer.trim().is_empty()
                        || !self.pending_files.is_empty();
                    let send = egui::Button::new(
                        egui::RichText::new("↑")
                            .size(20.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(accent_color())
                    .rounding(egui::Rounding::same(20.0))
                    .min_size(egui::vec2(40.0, 40.0));
                    if ui.add_enabled(can_send, send).on_hover_text("Send").clicked() {
                        self.send_current_message(workspace_id);
                    }
                });
            });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Drop files anywhere · ``code`` for code blocks")
                    .size(11.5)
                    .color(muted_color()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Files up to {} MB",
                        MAX_ATTACHMENT_BYTES / (1024 * 1024)
                    ))
                    .size(11.5)
                    .color(muted_color()),
                );
            });
        });
    }

    fn send_current_message(&mut self, workspace_id: &str) {
        if self.composer.trim().is_empty() && self.pending_files.is_empty() {
            return;
        }
        let text = self.composer.clone();
        let files = self.pending_files.clone();
        match self
            .db
            .send_message_with_files(workspace_id, &text, &files)
        {
            Ok(_) => {
                self.composer.clear();
                self.pending_files.clear();
                self.set_status("Message queued", false);
                self.start_sync();
            }
            Err(err) => self.set_status(format!("{err:#}"), true),
        }
    }

    fn add_pending_files(&mut self, files: Vec<PathBuf>) {
        for path in files {
            if !path.is_file() {
                continue;
            }
            if self.pending_files.iter().any(|existing| existing == &path) {
                continue;
            }
            match path.metadata() {
                Ok(metadata) if metadata.len() <= MAX_ATTACHMENT_BYTES => {
                    self.pending_files.push(path);
                }
                Ok(_) => self.set_status(
                    format!(
                        "{} is larger than the {} MB attachment limit",
                        display_file_name(&path),
                        MAX_ATTACHMENT_BYTES / (1024 * 1024)
                    ),
                    true,
                ),
                Err(err) => self.set_status(
                    format!("Could not inspect {}: {err}", display_file_name(&path)),
                    true,
                ),
            }
        }
    }

    fn message_row(
        &mut self,
        ui: &mut egui::Ui,
        workspace_id: &str,
        my_id: &str,
        message: MessageView,
    ) {
        let is_mine = message.author_id == my_id;
        let author_label = if is_mine {
            "You".to_string()
        } else {
            message.author_name.clone()
        };
        let avatar_fill = if is_mine { accent_color() } else { soft_color() };
        let avatar_text = if is_mine {
            egui::Color32::WHITE
        } else {
            primary_color()
        };
        let bubble_fill = if is_mine {
            accent_soft_color()
        } else {
            soft_color()
        };

        ui.horizontal_top(|ui| {
            egui::Frame::none()
                .fill(avatar_fill)
                .rounding(egui::Rounding::same(16.0))
                .inner_margin(7.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(initials(&author_label))
                            .size(12.0)
                            .strong()
                            .color(avatar_text),
                    );
                });

            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&author_label)
                            .strong()
                            .color(primary_color()),
                    );
                    ui.label(
                        egui::RichText::new(format_time(&message.created_at))
                            .small()
                            .color(muted_color()),
                    );
                    if message.edited_at.is_some() && !message.deleted {
                        ui.label(
                            egui::RichText::new("edited")
                                .small()
                                .color(muted_color()),
                        );
                    }

                    if is_mine && !message.deleted {
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("Delete").clicked() {
                                    match self.db.delete_message(workspace_id, &message.id) {
                                        Ok(_) => {
                                            self.set_status("Delete queued", false);
                                            self.start_sync();
                                        }
                                        Err(err) => self.set_status(format!("{err:#}"), true),
                                    }
                                }
                                if !message.body.is_empty() && ui.small_button("Edit").clicked() {
                                    self.edit_message_id = message.id.clone();
                                    self.edit_message_text = message.body.clone();
                                    self.show_edit = true;
                                }
                            },
                        );
                    }
                });
                ui.add_space(3.0);

                egui::Frame::none()
                    .fill(bubble_fill)
                    .rounding(egui::Rounding::same(16.0))
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_max_width(760.0);
                        if message.deleted {
                            ui.label(
                                egui::RichText::new("Message deleted")
                                    .italics()
                                    .color(muted_color()),
                            );
                            return;
                        }
                        if !message.body.is_empty() {
                            render_message_body(ui, &message.body);
                        }
                        match self.db.attachments_for_message(&message.id) {
                            Ok(attachments) => {
                                for attachment in attachments {
                                    self.attachment_row(ui, &attachment);
                                }
                            }
                            Err(err) => {
                                ui.small(format!("Could not load attachments: {err}"));
                            }
                        }
                    });
            });
        });
    }

    fn attachment_row(&mut self, ui: &mut egui::Ui, attachment: &AttachmentView) {
        ui.add_space(8.0);
        egui::Frame::none()
            .fill(surface_color())
            .stroke(egui::Stroke::new(1.0_f32, border_color()))
            .rounding(egui::Rounding::same(12.0))
            .inner_margin(9.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::Frame::none()
                        .fill(accent_soft_color())
                        .rounding(egui::Rounding::same(9.0))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("FILE")
                                    .size(10.0)
                                    .strong()
                                    .color(accent_color()),
                            );
                        });
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(&attachment.file_name)
                                .strong()
                                .color(primary_color()),
                        );
                        ui.label(
                            egui::RichText::new(format_file_size(attachment.size_bytes))
                                .small()
                                .color(muted_color()),
                        );
                    });
                    if attachment.upload_pending {
                        ui.label(
                            egui::RichText::new("Uploading…")
                                .small()
                                .color(accent_color()),
                        );
                    }

                    let local_path = attachment
                        .local_path
                        .as_ref()
                        .filter(|path| path.exists())
                        .cloned();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(path) = local_path {
                            if ui.small_button("Open").clicked() {
                                match open_path(&path) {
                                    Ok(_) => self.set_status("Opened file", false),
                                    Err(err) => self.set_status(err, true),
                                }
                            }
                        } else if self.downloading_attachment.as_deref()
                            == Some(attachment.id.as_str())
                        {
                            ui.label(
                                egui::RichText::new("Downloading…")
                                    .small()
                                    .color(accent_color()),
                            );
                        } else if ui.small_button("Download").clicked() {
                            self.start_attachment_download(attachment);
                        }
                    });
                });
            });
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        if self.selected_workspace.is_none() || !self.db.has_identity().unwrap_or(false) {
            return;
        }
        let files = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !files.is_empty() {
            self.add_pending_files(files);
        }
    }

    fn alert_toast(&mut self, ctx: &egui::Context) {
        let expired = self
            .alert_toast
            .as_ref()
            .map(|toast| toast.created_at.elapsed() > Duration::from_secs(5))
            .unwrap_or(false);
        if expired {
            self.alert_toast = None;
        }
        let Some(toast) = self.alert_toast.as_ref() else {
            return;
        };
        egui::Area::new(egui::Id::new("message_alert_toast"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 78.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(surface_color())
                    .stroke(egui::Stroke::new(1.0_f32, border_color()))
                    .rounding(egui::Rounding::same(16.0))
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.set_max_width(340.0);
                        ui.horizontal_top(|ui| {
                            egui::Frame::none()
                                .fill(accent_color())
                                .rounding(egui::Rounding::same(9.0))
                                .inner_margin(5.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("T")
                                            .strong()
                                            .color(egui::Color32::WHITE),
                                    );
                                });
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&toast.title)
                                        .strong()
                                        .color(primary_color()),
                                );
                                if !toast.body.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&toast.body)
                                            .color(muted_color()),
                                    );
                                }
                            });
                        });
                    });
            });
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.identity_window(ctx);
        self.add_person_window(ctx);
        self.new_workspace_window(ctx);
        self.people_window(ctx);
        self.share_workspace_window(ctx);
        self.join_workspace_window(ctx);
        self.edit_window(ctx);
        self.settings_window(ctx);
    }

    fn identity_window(&mut self, ctx: &egui::Context) {
        if !self.show_identity {
            return;
        }
        let mut open = true;
        egui::Window::new("Share my profile")
            .open(&mut open)
            .default_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                let networks = self.db.networks().unwrap_or_default();
                if self.share_profile_network_id.is_empty() {
                    if let Some(network) = networks.first() {
                        self.share_profile_network_id = network.network_id.clone();
                    }
                }
                ui.label(
                    egui::RichText::new("Choose the Firebase connection this profile should use.")
                        .color(muted_color()),
                );
                ui.add_space(6.0);
                let selected_label = networks
                    .iter()
                    .find(|network| network.network_id == self.share_profile_network_id)
                    .map(|network| network.label.clone())
                    .unwrap_or_else(|| "Choose database".to_string());
                egui::ComboBox::from_id_salt("share_profile_network")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        for network in &networks {
                            ui.selectable_value(
                                &mut self.share_profile_network_id,
                                network.network_id.clone(),
                                &network.label,
                            );
                        }
                    });
                ui.add_space(10.0);
                match self
                    .db
                    .contact_invite_code_for_network(&self.share_profile_network_id)
                {
                    Ok(code) => {
                        let mut shown = code.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut shown)
                                .desired_rows(6)
                                .interactive(false),
                        );
                        ui.label(
                            egui::RichText::new(
                                "The recipient automatically adds this Firebase connection when they add you.",
                            )
                            .small()
                            .color(muted_color()),
                        );
                        ui.add_space(8.0);
                        if ui
                            .add(
                                egui::Button::new("Copy profile code")
                                    .fill(accent_color())
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding::same(12.0)),
                            )
                            .clicked()
                        {
                            ui.output_mut(|output| output.copied_text = code.clone());
                            self.set_status("Profile code copied", false);
                        }
                    }
                    Err(err) => {
                        ui.label(format!("{err:#}"));
                    }
                }
            });
        self.show_identity = open;
    }

    fn add_person_window(&mut self, ctx: &egui::Context) {
        if !self.show_add_person {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Add a contact")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Paste the profile code they shared with you.")
                        .color(muted_color()),
                );
                ui.add_space(6.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.contact_code)
                        .desired_rows(6)
                        .hint_text("TRIXY-CONTACT2-…"),
                );
                ui.label(
                    egui::RichText::new(
                        "New profile codes include their Firebase route, so you do not need to configure it first.",
                    )
                    .small()
                    .color(muted_color()),
                );
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Button::new("Add contact")
                            .fill(accent_color())
                            .stroke(egui::Stroke::NONE)
                            .rounding(egui::Rounding::same(12.0)),
                    )
                    .clicked()
                {
                    match self.db.import_contact_code(&self.contact_code) {
                        Ok(user) => {
                            self.set_status(format!("Added {}", user.name), false);
                            self.contact_code.clear();
                            self.sidebar_mode = SidebarMode::Contacts;
                            close = true;
                        }
                        Err(err) => self.set_status(format!("{err:#}"), true),
                    }
                }
            });
        self.show_add_person = open && !close;
    }

    fn new_workspace_window(&mut self, ctx: &egui::Context) {
        if !self.show_new_workspace {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("New workspace")
            .open(&mut open)
            .default_width(430.0)
            .show(ctx, |ui| {
                let networks = self.db.networks().unwrap_or_default();
                if self.new_workspace_network_id.is_empty() {
                    if let Some(network) = networks.first() {
                        self.new_workspace_network_id = network.network_id.clone();
                    }
                }
                ui.label(egui::RichText::new("Name").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_workspace_name)
                        .hint_text("Project or team name"),
                );
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Firebase connection").strong());
                let selected_label = networks
                    .iter()
                    .find(|network| network.network_id == self.new_workspace_network_id)
                    .map(|network| network.label.clone())
                    .unwrap_or_else(|| "Choose database".to_string());
                egui::ComboBox::from_id_salt("new_workspace_network")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        for network in &networks {
                            ui.selectable_value(
                                &mut self.new_workspace_network_id,
                                network.network_id.clone(),
                                &network.label,
                            );
                        }
                    });
                ui.label(
                    egui::RichText::new("Each workspace uses one Firebase database; Trixy can stay connected to many at once.")
                        .small()
                        .color(muted_color()),
                );
                ui.add_space(10.0);
                if networks.is_empty() {
                    ui.label("Add a Firebase connection in Settings first.");
                } else if ui
                    .add(
                        egui::Button::new("Create workspace")
                            .fill(accent_color())
                            .stroke(egui::Stroke::NONE)
                            .rounding(egui::Rounding::same(12.0)),
                    )
                    .clicked()
                {
                    match self.db.create_workspace_on_network(
                        &self.new_workspace_name,
                        &self.new_workspace_network_id,
                    ) {
                        Ok(id) => {
                            self.selected_workspace = Some(id);
                            self.new_workspace_name.clear();
                            self.sidebar_mode = SidebarMode::Workspaces;
                            self.set_status("Workspace created", false);
                            close = true;
                        }
                        Err(err) => self.set_status(format!("{err:#}"), true),
                    }
                }
            });
        self.show_new_workspace = open && !close;
    }

    fn people_window(&mut self, ctx: &egui::Context) {
        if !self.show_people {
            return;
        }
        let Some(workspace_id) = self.selected_workspace.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Workspace people")
            .open(&mut open)
            .default_width(500.0)
            .show(ctx, |ui| {
                let members = self.db.members(&workspace_id).unwrap_or_default();
                let network = self.db.workspace_network(&workspace_id).ok();
                ui.horizontal(|ui| {
                    ui.heading("People");
                    if let Some(network) = &network {
                        pill_frame(accent_soft_color()).show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&network.label)
                                    .small()
                                    .color(accent_color()),
                            );
                        });
                    }
                });
                ui.label(
                    egui::RichText::new("Everyone in this workspace is also available as a Trixy contact.")
                        .small()
                        .color(muted_color()),
                );
                ui.add_space(8.0);
                for member in &members {
                    egui::Frame::none()
                        .fill(soft_color())
                        .rounding(egui::Rounding::same(12.0))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                avatar(ui, &member.name, 30.0, false);
                                ui.label(egui::RichText::new(&member.name).strong());
                            });
                        });
                    ui.add_space(4.0);
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Add someone");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Share workspace link").clicked() {
                            self.workspace_share_link.clear();
                            self.show_share_workspace = true;
                        }
                    });
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.workspace_contact_search)
                        .hint_text("Search contacts…"),
                );
                let member_ids: std::collections::HashSet<String> =
                    members.iter().map(|member| member.user_id.clone()).collect();
                let query = self.workspace_contact_search.trim().to_lowercase();
                let contacts = if let Some(network) = &network {
                    self.db.contacts_for_network(&network.network_id).unwrap_or_default()
                } else {
                    Vec::new()
                };
                for contact in contacts {
                    if !query.is_empty() && !contact.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        avatar(ui, &contact.name, 28.0, false);
                        ui.label(&contact.name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if member_ids.contains(&contact.user_id) {
                                ui.label(
                                    egui::RichText::new("Already here")
                                        .small()
                                        .color(muted_color()),
                                );
                            } else if ui.small_button("Add").clicked() {
                                match self.db.add_member(&workspace_id, &contact.user_id) {
                                    Ok(_) => {
                                        self.set_status(
                                            format!("Added {} to workspace", contact.name),
                                            false,
                                        );
                                        self.start_sync();
                                    }
                                    Err(err) => self.set_status(format!("{err:#}"), true),
                                }
                            }
                        });
                    });
                }
            });
        self.show_people = open;
    }

    fn share_workspace_window(&mut self, ctx: &egui::Context) {
        if !self.show_share_workspace {
            return;
        }
        let Some(workspace_id) = self.selected_workspace.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Share workspace")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                let name = self
                    .db
                    .workspace_name(&workspace_id)
                    .unwrap_or_else(|_| "Workspace".to_string());
                ui.label(
                    egui::RichText::new(&name)
                        .size(20.0)
                        .strong()
                        .color(primary_color()),
                );
                ui.label(
                    egui::RichText::new(
                        "Anyone with this encrypted link can join. Trixy imports the workspace and automatically adds every existing member to their Contacts.",
                    )
                    .color(muted_color()),
                );
                ui.add_space(10.0);

                if self.workspace_share_link.is_empty() {
                    if self.sharing_workspace {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Creating encrypted workspace link…");
                        });
                    } else if ui
                        .add(
                            egui::Button::new("Create workspace link")
                                .fill(accent_color())
                                .stroke(egui::Stroke::NONE)
                                .rounding(egui::Rounding::same(12.0)),
                        )
                        .clicked()
                    {
                        self.start_workspace_share(&workspace_id);
                    }
                } else {
                    let mut shown = self.workspace_share_link.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .desired_rows(5)
                            .interactive(false),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new("Copy link")
                                    .fill(accent_color())
                                    .stroke(egui::Stroke::NONE)
                                    .rounding(egui::Rounding::same(12.0)),
                            )
                            .clicked()
                        {
                            ui.output_mut(|output| {
                                output.copied_text = self.workspace_share_link.clone()
                            });
                            self.set_status("Workspace link copied", false);
                        }
                        if ui.button("Create a fresh link").clicked() {
                            self.start_workspace_share(&workspace_id);
                        }
                    });
                    ui.label(
                        egui::RichText::new(
                            "Treat the link like an invitation secret. It contains the capability needed to join the encrypted workspace.",
                        )
                        .small()
                        .color(muted_color()),
                    );
                }
            });
        self.show_share_workspace = open;
    }

    fn join_workspace_window(&mut self, ctx: &egui::Context) {
        if !self.show_join_workspace {
            return;
        }
        let mut open = true;
        egui::Window::new("Join a workspace")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Paste a Trixy workspace link. Its Firebase database is added automatically, and all workspace members are imported into Contacts.",
                    )
                    .color(muted_color()),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.join_workspace_code)
                        .desired_rows(6)
                        .hint_text("trixy://join/…"),
                );
                ui.add_space(8.0);
                if self.joining_workspace {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Joining workspace…");
                    });
                } else if ui
                    .add(
                        egui::Button::new("Join workspace")
                            .fill(accent_color())
                            .stroke(egui::Stroke::NONE)
                            .rounding(egui::Rounding::same(12.0)),
                    )
                    .clicked()
                {
                    self.start_join_workspace();
                }
            });
        self.show_join_workspace = open;
    }

    fn edit_window(&mut self, ctx: &egui::Context) {
        if !self.show_edit {
            return;
        }
        let Some(workspace_id) = self.selected_workspace.clone() else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new("Edit message")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.edit_message_text).desired_rows(6),
                );
                ui.small("Code blocks use matching double backticks: ``code``");
                if ui.button("Save changes").clicked() {
                    match self.db.edit_message(
                        &workspace_id,
                        &self.edit_message_id,
                        &self.edit_message_text,
                    ) {
                        Ok(_) => {
                            self.set_status("Edit queued", false);
                            self.start_sync();
                            close = true;
                        }
                        Err(err) => self.set_status(format!("{err:#}"), true),
                    }
                }
            });
        self.show_edit = open && !close;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        egui::Window::new("Trixy settings")
            .open(&mut open)
            .default_width(620.0)
            .resizable(true)
            .show(ctx, |ui| {
                if let Ok(identity) = self.db.identity() {
                    ui.horizontal(|ui| {
                        paint_logo(ui, 44.0);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&identity.name)
                                    .size(18.0)
                                    .strong()
                                    .color(primary_color()),
                            );
                            ui.label(
                                egui::RichText::new("Your Trixy identity lives on this computer")
                                    .small()
                                    .color(muted_color()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Share my profile").clicked() {
                                self.show_identity = true;
                            }
                        });
                    });
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(10.0);
                ui.heading("Firebase connections");
                ui.label(
                    egui::RichText::new(
                        "Trixy can stay connected to several Realtime Database URLs at the same time. Each workspace chooses one connection.",
                    )
                    .color(muted_color()),
                );
                ui.add_space(8.0);

                let networks = self.db.networks().unwrap_or_default();
                for network in &networks {
                    egui::Frame::none()
                        .fill(soft_color())
                        .rounding(egui::Rounding::same(14.0))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&network.label)
                                            .strong()
                                            .color(primary_color()),
                                    );
                                    ui.label(
                                        egui::RichText::new(&network.firebase_url)
                                            .small()
                                            .color(muted_color()),
                                    );
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        egui::RichText::new("● Configured")
                                            .small()
                                            .color(success_color()),
                                    );
                                });
                            });
                        });
                    ui.add_space(5.0);
                }

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Add another database").strong());
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [160.0, 36.0],
                        egui::TextEdit::singleline(&mut self.new_network_label)
                            .hint_text("Label, e.g. Lab"),
                    );
                    ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::TextEdit::singleline(&mut self.new_network_url)
                            .hint_text("https://…-default-rtdb.firebaseio.com"),
                    );
                });
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new("Add connection")
                                .fill(accent_soft_color())
                                .stroke(egui::Stroke::NONE)
                                .rounding(egui::Rounding::same(12.0)),
                        )
                        .clicked()
                    {
                        match self
                            .db
                            .add_network(&self.new_network_label, &self.new_network_url)
                        {
                            Ok(network) => {
                                self.new_network_label.clear();
                                self.new_network_url.clear();
                                self.new_workspace_network_id = network.network_id.clone();
                                self.share_profile_network_id = network.network_id;
                                self.set_status("Firebase connection added", false);
                                self.start_sync();
                            }
                            Err(err) => self.set_status(format!("{err:#}"), true),
                        }
                    }
                    ui.label(
                        egui::RichText::new("Use the same firebase-rules.json on every database.")
                            .small()
                            .color(muted_color()),
                    );
                });

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(10.0);
                ui.heading("Files & notifications");
                ui.label(
                    egui::RichText::new(format!(
                        "Encrypted attachments use {} KB chunks with retry. Maximum file size: {} MB.",
                        ATTACHMENT_CHUNK_SIZE / 1024,
                        MAX_ATTACHMENT_BYTES / (1024 * 1024)
                    ))
                    .small()
                    .color(muted_color()),
                );
                ui.label(
                    egui::RichText::new(
                        "Trixy only makes outbound HTTPS requests; it does not open inbound ports or require a VPN.",
                    )
                    .small()
                    .color(muted_color()),
                );
            });
        self.show_settings = open;
    }

}

impl eframe::App for TrixyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_probe();
        self.poll_sync(ctx);
        self.poll_attachment_download();
        self.poll_workspace_share();
        self.poll_join_workspace();
        self.handle_dropped_files(ctx);

        if self.db.has_identity().unwrap_or(false)
            && !self.syncing
            && self.last_sync_start.elapsed() >= Duration::from_secs(2)
        {
            self.start_sync();
        }

        if !self.db.has_identity().unwrap_or(false) {
            self.setup_ui(ctx);
        } else {
            self.top_bar(ctx);
            self.workspace_sidebar(ctx);
            self.composer_panel(ctx);
            self.conversation_ui(ctx);
            self.windows(ctx);
            self.alert_toast(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn render_message_body(ui: &mut egui::Ui, text: &str) {
    let mut cursor = 0usize;
    while cursor < text.len() {
        let Some(relative) = text[cursor..].find("``") else {
            ui.label(&text[cursor..]);
            break;
        };
        let open = cursor + relative;
        let fence = if text[open..].starts_with("```") {
            "```"
        } else {
            "``"
        };
        let content_start = open + fence.len();
        let Some(close_relative) = text[content_start..].find(fence) else {
            ui.label(&text[cursor..]);
            break;
        };
        let close = content_start + close_relative;
        if open > cursor {
            ui.label(&text[cursor..open]);
        }
        let code = text[content_start..close].trim_matches('\n');
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(232, 232, 237))
            .stroke(egui::Stroke::new(1.0_f32, border_color()))
            .rounding(egui::Rounding::same(10.0))
            .inner_margin(11.0)
            .show(ui, |ui| { ui.monospace(code); });
        cursor = close + fence.len();
    }
    if text.is_empty() {
        ui.label("");
    }
}

fn initials(name: &str) -> String {
    let mut parts = name
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return "?".to_string();
    }
    if parts.len() == 1 {
        return parts.remove(0).to_uppercase().collect();
    }
    parts.into_iter().collect::<String>().to_uppercase()
}

fn compact_preview(text: &str, max_chars: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= max_chars { return cleaned; }
    let mut out = cleaned.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    out.push('…');
    out
}

fn play_message_alert(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let title = applescript_escape(title);
        let body = applescript_escape(body);
        let script = format!(
            "display notification \"{}\" with title \"Trixy\" subtitle \"{}\"",
            body, title
        );
        let _ = Command::new("/usr/bin/afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .spawn();
        let _ = Command::new("osascript").args(["-e", &script]).spawn();
    }

    #[cfg(target_os = "windows")]
    unsafe {
        MessageBeep(0x00000040);
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = (title, body);
    }
}

#[cfg(target_os = "macos")]
fn applescript_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\"', "\\\"")
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn MessageBeep(u_type: u32) -> i32;
}

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string()
}

fn format_file_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(path).spawn();

    #[cfg(target_os = "windows")]
    let result = Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let result = Command::new("xdg-open").arg(path).spawn();

    result
        .map(|_| ())
        .map_err(|err| format!("Could not open {}: {err}", path.display()))
}
