use crate::theme::*;
use crate::utils::cmd_button;
use eframe::egui;
use egui_taffy::bg::simple::{TuiBackground, TuiBuilderLogicWithBackground};
use egui_taffy::taffy::prelude::{length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};

pub struct HeaderResponse {
    pub update_clicked: bool,
}

pub fn draw<'a>(
    tui: impl TuiBuilderLogic<'a>,
    is_busy: bool,
    version: &str,
    update_button_label: Option<&str>,
    update_button_enabled: bool,
) -> HeaderResponse {
    let mut update_clicked = false;
    let version_text = format!("v{version}");

    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Row,
        justify_content: Some(taffy::JustifyContent::SpaceBetween),
        align_items: Some(taffy::AlignItems::Center),
        padding: length(6.0),
        size: taffy::Size {
            width: percent(1.),
            height: percent(1.),
        },
        ..Default::default()
    })
    .bg_add(
        TuiBackground::new()
            .with_background_color(COL_BG)
            .with_border_color(COL_BORDER)
            .with_border_width(1.0),
        |tui| {
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                align_items: Some(taffy::AlignItems::Center),
                gap: length(8.0),
                ..Default::default()
            })
            .add(|tui| {
                tui.label(
                    egui::RichText::new("FLEET")
                        .family(egui::FontFamily::Monospace)
                        .size(12.0)
                        .extra_letter_spacing(2.0)
                        .strong()
                        .color(COL_TEXT),
                );
                tui.label(
                    egui::RichText::new(version_text)
                        .family(egui::FontFamily::Monospace)
                        .size(10.0)
                        .color(COL_TEXT_DIM),
                );
            });

            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                align_items: Some(taffy::AlignItems::Center),
                gap: length(6.0),
                ..Default::default()
            })
            .add(|tui| {
                if let Some(label) = update_button_label {
                    let resp = tui.ui(|ui| cmd_button(ui, label, "primary", update_button_enabled));
                    update_clicked |= resp.clicked();
                }

                if is_busy {
                    tui.ui_add(egui::Spinner::new());
                    tui.label(
                        egui::RichText::new("STATUS: BUSY")
                            .color(COL_WARN)
                            .size(10.0),
                    );
                } else {
                    tui.label(
                        egui::RichText::new("STATUS: IDLE")
                            .color(COL_ACCENT)
                            .size(10.0),
                    );
                }
            });
        },
    );

    HeaderResponse { update_clicked }
}
