use crate::theme::*;
use crate::utils::{cmd_button, section_label};
use eframe::egui;
use egui_taffy::bg::simple::{TuiBackground, TuiBuilderLogicWithBackground};
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::viewmodel::DashboardState;

pub struct CommandInterfaceResponse {
    pub sync: bool,
    pub check_remote: bool,
    pub check_local: bool,
    pub repair: bool,
    pub launch: bool,
    pub join: bool,
    pub cancel: bool,
    pub ack: bool,
}

pub fn draw<'a>(tui: impl TuiBuilderLogic<'a>, state: &DashboardState) -> CommandInterfaceResponse {
    let mut resp = CommandInterfaceResponse {
        sync: false,
        check_remote: false,
        check_local: false,
        repair: false,
        launch: false,
        join: false,
        cancel: false,
        ack: false,
    };

    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(4.0),
        size: taffy::Size {
            width: percent(1.),
            height: auto(),
        },
        ..Default::default()
    })
    .add(|tui| {
        tui.ui(|ui| section_label(ui, "COMMAND"));

        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Column,
            padding: length(12.0),
            gap: length(6.0),
            size: taffy::Size {
                width: percent(1.),
                height: auto(),
            },
            ..Default::default()
        })
        .bg_add(
            TuiBackground::new()
                .with_background_color(COL_BG_DARK)
                .with_border_color(COL_BORDER)
                .with_border_width(1.0),
            |tui| {
                let (mode_text, is_busy) = match state {
                    DashboardState::Idle { .. } => ("IDLE", false),
                    DashboardState::Busy { .. } => ("BUSY", true),
                    DashboardState::Review { .. } => ("REVIEW", false),
                    DashboardState::Synced { .. } => ("SYNCED", false),
                    DashboardState::Error { .. } => ("ERROR", false),
                    DashboardState::Unknown { .. } => ("UNKNOWN", false),
                };

                tui.style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Row,
                    justify_content: Some(taffy::JustifyContent::SpaceBetween),
                    align_items: Some(taffy::AlignItems::Center),
                    ..Default::default()
                })
                .add(|tui| {
                    tui.label(
                        egui::RichText::new(format!("MODE: {mode_text}")).color(COL_TEXT_DIM),
                    );
                    if is_busy {
                        tui.style(taffy::Style {
                            flex_direction: taffy::FlexDirection::Row,
                            gap: length(6.0),
                            align_items: Some(taffy::AlignItems::Center),
                            ..Default::default()
                        })
                        .add(|tui| {
                            tui.ui_add(egui::Spinner::new());
                            tui.label(egui::RichText::new("WORK").color(COL_WARN));
                        });
                    }
                });

                let task_lbl = match state {
                    DashboardState::Busy { task_name, .. } => task_name.clone(),
                    DashboardState::Review {
                        changes_summary, ..
                    } => changes_summary.clone(),
                    DashboardState::Synced { .. } => "UP TO DATE".to_string(),
                    DashboardState::Error { msg } => msg.clone(),
                    DashboardState::Idle { .. } => "READY".to_string(),
                    DashboardState::Unknown { msg } => msg.clone(),
                };

                tui.style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Row,
                    gap: length(6.0),
                    align_items: Some(taffy::AlignItems::Center),
                    ..Default::default()
                })
                .add(|tui| {
                    tui.label(egui::RichText::new("TASK:").color(COL_TEXT_DIM));
                    tui.label(
                        egui::RichText::new(task_lbl)
                            .color(COL_TEXT)
                            .strong()
                            .monospace(),
                    );
                });

                let detail_lbl = match state {
                    DashboardState::Busy { detail, .. } => Some(detail.as_str()),
                    DashboardState::Synced { msg, .. } => Some(msg.as_str()),
                    DashboardState::Error { msg } => Some(msg.as_str()),
                    DashboardState::Unknown { msg } => Some(msg.as_str()),
                    _ => None,
                };

                if let Some(detail_lbl) = detail_lbl {
                    tui.style(taffy::Style {
                        flex_direction: taffy::FlexDirection::Row,
                        gap: length(6.0),
                        align_items: Some(taffy::AlignItems::Center),
                        ..Default::default()
                    })
                    .add(|tui| {
                        tui.label(egui::RichText::new("INFO:").color(COL_TEXT_DIM));
                        tui.label(egui::RichText::new(detail_lbl).color(COL_TEXT_DIM));
                    });
                }

                let (prog_val, is_active) = match state {
                    DashboardState::Busy { progress, .. } => {
                        (progress.as_ref().map(|p| p.0).unwrap_or(0.0), true)
                    }
                    DashboardState::Synced { .. } => (1.0, true),
                    _ => (0.0, false),
                };

                let prog_lbl = match state {
                    DashboardState::Busy { progress, .. } => {
                        progress.as_ref().map(|p| p.1.as_str())
                    }
                    _ => None,
                };

                tui.style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Column,
                    gap: length(2.0),
                    ..Default::default()
                })
                .add(|tui| {
                    tui.label(egui::RichText::new("PROG:").color(COL_TEXT_DIM));
                    tui.style(taffy::Style {
                        size: taffy::Size {
                            width: percent(1.),
                            height: length(4.0),
                        },
                        ..Default::default()
                    })
                    .ui(|ui| {
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 0.0, COL_BORDER);
                        if is_active {
                            let fill_w = rect.width() * prog_val;
                            let fill_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(fill_w, rect.height()),
                            );
                            ui.painter().rect_filled(fill_rect, 0.0, COL_ACCENT);
                        }
                    });

                    if let Some(prog_lbl) = prog_lbl {
                        if !prog_lbl.trim().is_empty() {
                            tui.label(egui::RichText::new(prog_lbl).color(COL_TEXT_DIM));
                        }
                    }
                });

                tui.style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Row,
                    gap: length(8.0),
                    margin: taffy::Rect {
                        left: length(0.0),
                        right: length(0.0),
                        top: length(6.0),
                        bottom: length(0.0),
                    },
                    ..Default::default()
                })
                .add(|tui| match state {
                    DashboardState::Busy { can_cancel, .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "CANCEL", "danger", *can_cancel))
                            .clicked()
                        {
                            resp.cancel = true;
                        }
                    }
                    DashboardState::Review { can_launch, .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "SYNC", "primary", true))
                            .clicked()
                        {
                            resp.sync = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "LAUNCH", "outline", *can_launch))
                            .clicked()
                        {
                            resp.launch = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "JOIN", "outline", *can_launch))
                            .clicked()
                        {
                            resp.join = true;
                        }
                    }
                    DashboardState::Synced { can_launch, .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "LAUNCH", "primary", *can_launch))
                            .clicked()
                        {
                            resp.launch = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "JOIN", "outline", *can_launch))
                            .clicked()
                        {
                            resp.join = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "LOCAL CHECK", "outline", true))
                            .clicked()
                        {
                            resp.check_local = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "CHECK FOR UPDATES", "outline", true))
                            .clicked()
                        {
                            resp.check_remote = true;
                        }
                    }
                    DashboardState::Error { .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "ACK", "outline", true))
                            .clicked()
                        {
                            resp.ack = true;
                        }
                    }
                    DashboardState::Unknown { .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "REPAIR", "primary", true))
                            .clicked()
                        {
                            resp.repair = true;
                        }
                    }
                    DashboardState::Idle { can_launch, .. } => {
                        if tui
                            .ui(|ui| cmd_button(ui, "LAUNCH", "primary", *can_launch))
                            .clicked()
                        {
                            resp.launch = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "JOIN", "outline", *can_launch))
                            .clicked()
                        {
                            resp.join = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "LOCAL CHECK", "outline", true))
                            .clicked()
                        {
                            resp.check_local = true;
                        }
                        if tui
                            .ui(|ui| cmd_button(ui, "CHECK FOR UPDATES", "outline", true))
                            .clicked()
                        {
                            resp.check_remote = true;
                        }
                    }
                });
            },
        );
    });

    resp
}
