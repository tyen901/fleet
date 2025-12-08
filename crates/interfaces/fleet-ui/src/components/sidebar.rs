use crate::theme::*;
use crate::utils::{cmd_button, section_label};
use eframe::egui;
use egui_taffy::bg::simple::{TuiBackground, TuiBuilderLogicWithBackground};
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::domain::ProfileId;
use fleet_app_core::viewmodel::ProfileHubVm;

pub struct SidebarResponse {
    pub selected_id: Option<ProfileId>,
    pub add_clicked: bool,
    pub settings_clicked: bool,
}

pub fn draw<'a>(
    tui: impl TuiBuilderLogic<'a>,
    vm: &ProfileHubVm,
    selected_id: Option<ProfileId>,
) -> SidebarResponse {
    let mut resp = SidebarResponse {
        selected_id: None,
        add_clicked: false,
        settings_clicked: false,
    };

    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        size: percent(1.),
        min_size: taffy::Size {
            width: percent(1.),
            height: length(0.0),
        },
        justify_content: Some(taffy::JustifyContent::SpaceBetween),
        align_items: Some(taffy::AlignItems::Stretch),
        padding: length(8.0),
        gap: length(8.0),
        ..Default::default()
    })
    .bg_add(
        TuiBackground::new()
            .with_background_color(COL_BG)
            .with_border_color(COL_BORDER)
            .with_border_width(1.0),
        |tui| {
            // Top region: header + scrolling list
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: length(0.0),
                min_size: taffy::Size {
                    width: percent(1.),
                    height: length(0.0),
                },
                gap: length(8.0),
                ..Default::default()
            })
            .add(|tui| {
                tui.ui(|ui| section_label(ui, "PROFILES"));

                tui.style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Column,
                    flex_grow: 1.0,
                    flex_basis: length(0.0),
                    min_size: taffy::Size {
                        width: percent(1.),
                        height: length(0.0),
                    },
                    overflow: taffy::Point {
                        x: taffy::Overflow::Hidden,
                        y: taffy::Overflow::Scroll,
                    },
                    gap: length(4.0),
                    size: taffy::Size {
                        width: percent(1.),
                        height: auto(),
                    },
                    ..Default::default()
                })
                .add(|tui| {
                    for profile in &vm.profiles {
                        let is_selected = Some(profile.id.clone()) == selected_id;
                        let badge_col = match profile.status_label.as_str() {
                            "Ready" => COL_SUCCESS,
                            "Update Available" => COL_ACCENT,
                            _ => COL_TEXT_DIM,
                        };

                        let response = tui
                            .id(egui_taffy::tid(("profile", &profile.id)))
                            .style(taffy::Style {
                                flex_direction: taffy::FlexDirection::Row,
                                align_items: Some(taffy::AlignItems::Center),
                                size: taffy::Size {
                                    width: percent(1.),
                                    height: length(32.0),
                                },
                                padding: length(4.0),
                                gap: length(8.0),
                                ..Default::default()
                            })
                            .bg_clickable(
                                TuiBackground::new()
                                    .with_background_color(if is_selected {
                                        COL_ACCENT.linear_multiply(0.1)
                                    } else {
                                        COL_BG
                                    })
                                    .with_border_color(if is_selected {
                                        COL_ACCENT
                                    } else {
                                        COL_BORDER
                                    })
                                    .with_border_width(1.0),
                                |tui| {
                                    if is_selected {
                                        tui.style(taffy::Style {
                                            size: taffy::Size {
                                                width: length(2.0),
                                                height: percent(1.),
                                            },
                                            flex_shrink: 0.0,
                                            ..Default::default()
                                        })
                                        .bg_add(
                                            TuiBackground::new().with_background_color(COL_ACCENT),
                                            |_| {},
                                        );
                                    }

                                    tui.style(taffy::Style {
                                        size: taffy::Size {
                                            width: length(6.0),
                                            height: length(6.0),
                                        },
                                        flex_shrink: 0.0,
                                        ..Default::default()
                                    })
                                    .bg_add(
                                        TuiBackground::new()
                                            .with_background_color(badge_col)
                                            .with_corner_radius(3.0),
                                        |_| {},
                                    );

                                    tui.label(
                                        egui::RichText::new(&profile.name)
                                            .size(12.0)
                                            .color(COL_TEXT)
                                            .monospace(),
                                    );
                                },
                            );

                        if response.clicked() {
                            resp.selected_id = Some(profile.id.clone());
                        }
                    }
                });
            });

            // Footer region: pinned buttons
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Column,
                gap: length(4.0),
                flex_shrink: 0.0,
                padding: length(4.0),
                ..Default::default()
            })
            .bg_add(
                TuiBackground::new()
                    .with_background_color(COL_BG_DARK)
                    .with_border_color(COL_BORDER)
                    .with_border_width(1.0),
                |tui| {
                    if tui
                        .ui(|ui| cmd_button(ui, "SETTINGS", "outline", true))
                        .clicked()
                    {
                        resp.settings_clicked = true;
                    }
                    if tui
                        .ui(|ui| cmd_button(ui, "ADD PROFILE", "primary", vm.can_create_profile))
                        .clicked()
                    {
                        resp.add_clicked = true;
                    }
                },
            );
        },
    );

    resp
}
