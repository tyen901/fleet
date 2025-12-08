use crate::theme::*;
use crate::utils::section_label;
use eframe::egui;
use egui_taffy::bg::simple::{TuiBackground, TuiBuilderLogicWithBackground};
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::viewmodel::ProfileStatsVm;

pub fn draw<'a>(tui: impl TuiBuilderLogic<'a>, stats: &Option<ProfileStatsVm>) {
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
        tui.ui(|ui| section_label(ui, "READOUT"));

        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Row,
            gap: length(8.0),
            padding: length(4.0),
            size: taffy::Size {
                width: percent(1.),
                height: length(52.0),
            },
            align_items: Some(taffy::AlignItems::Stretch),
            ..Default::default()
        })
        .bg_add(
            TuiBackground::new()
                .with_background_color(COL_BG_DARK)
                .with_border_color(COL_BORDER)
                .with_border_width(1.0),
            |tui| {
                draw_cell(
                    &mut *tui,
                    "SIZE",
                    stats
                        .as_ref()
                        .map(|s| s.total_size.as_str())
                        .unwrap_or("0 B"),
                );
                draw_cell(
                    &mut *tui,
                    "FILES",
                    stats.as_ref().map(|s| s.file_count.as_str()).unwrap_or("0"),
                );
                draw_cell(
                    &mut *tui,
                    "CACHE",
                    stats
                        .as_ref()
                        .map(|s| s.cache_ratio.as_str())
                        .unwrap_or("0%"),
                );
            },
        );
    });
}

fn draw_cell<'a>(tui: impl TuiBuilderLogic<'a>, label: &str, value: &str) {
    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        flex_grow: 1.0,
        gap: length(2.0),
        justify_content: Some(taffy::JustifyContent::Center),
        padding: length(4.0),
        ..Default::default()
    })
    .add(|tui| {
        tui.label(
            egui::RichText::new(label)
                .size(9.0)
                .color(COL_TEXT_DIM)
                .strong(),
        );
        tui.label(
            egui::RichText::new(value)
                .size(12.0)
                .color(COL_ACCENT)
                .monospace(),
        );
    });
}
