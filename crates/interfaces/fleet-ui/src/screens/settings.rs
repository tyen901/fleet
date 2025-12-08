use crate::utils::{cmd_button, section_label};
use eframe::egui;
use egui_taffy::taffy::prelude::{auto, length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::{
    domain::{FlatpakSteamAvailability, FLATPAK_STEAM_LAUNCH_TEMPLATE, STEAM_LAUNCH_TEMPLATE},
    viewmodel::settings_vm,
    FleetApplication, Route,
};

pub fn draw<'a>(tui: impl TuiBuilderLogic<'a>, app: &mut FleetApplication) {
    let vm = settings_vm(&app.state);
    let mut save_settings: Option<fleet_app_core::AppSettings> = None;
    let mut cancel_clicked = false;

    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(8.0),
        size: percent(1.),
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        },
        ..Default::default()
    })
    .add(|tui| {
        {
            let settings = app
                .state
                .settings_draft
                .get_or_insert_with(|| vm.settings.clone());

        tui.ui(|ui| section_label(ui, "SETTINGS"));

        tui.ui(|ui| section_label(ui, "NETWORK"));

        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Row,
            gap: length(6.0),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .add(|tui| {
            tui.label("Threads:");
            tui.ui_add(egui::DragValue::new(&mut settings.max_threads).range(1..=32));
        });

        tui.ui_add(egui::Checkbox::new(
            &mut settings.speed_limit_enabled,
            "Enable Speed Limit",
        ));

        if settings.speed_limit_enabled {
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                gap: length(6.0),
                align_items: Some(taffy::AlignItems::Center),
                ..Default::default()
            })
            .add(|tui| {
                tui.label("Bytes/s:");
                tui.ui_add(egui::DragValue::new(&mut settings.max_speed_bytes).speed(1024.0));
            });
        }

        tui.ui(|ui| section_label(ui, "LAUNCHER"));

        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Row,
            gap: length(6.0),
            align_items: Some(taffy::AlignItems::Center),
            ..Default::default()
        })
        .add(|tui| {
            tui.label("Launch method:");
            tui.ui(|ui| {
                let is_steam_selected = settings.launch_template == STEAM_LAUNCH_TEMPLATE;
                let is_flatpak_selected =
                    settings.launch_template == FLATPAK_STEAM_LAUNCH_TEMPLATE;

                let flatpak_available = matches!(
                    app.state.flatpak_steam,
                    FlatpakSteamAvailability::Available
                );

                let mut selected = if is_steam_selected {
                    0
                } else if is_flatpak_selected {
                    1
                } else {
                    -1
                };
                ui.horizontal(|ui| {
                    ui.radio_value(&mut selected, 0, "Steam");
                    ui.radio_value(&mut selected, 1, "Steam (Flatpak)");
                });

                if selected == 0 && !is_steam_selected {
                    settings.launch_template = STEAM_LAUNCH_TEMPLATE.to_string();
                }
                if selected == 1 && !is_flatpak_selected {
                    settings.launch_template = FLATPAK_STEAM_LAUNCH_TEMPLATE.to_string();
                }

                match &app.state.flatpak_steam {
                    FlatpakSteamAvailability::Unknown => {
                        ui.add_space(4.0);
                        ui.label("Flatpak Steam availability: unknown (selection still allowed).");
                    }
                    FlatpakSteamAvailability::Unavailable(reason) => {
                        ui.add_space(4.0);
                        ui.label(format!(
                            "Flatpak Steam unavailable: {reason}. Install `com.valvesoftware.Steam` or use the Steam option."
                        ));
                    }
                    FlatpakSteamAvailability::Available => {}
                }

                if !is_steam_selected && !is_flatpak_selected {
                    ui.add_space(4.0);
                    ui.label("Current launch template is custom; select an option above to switch back.");
                }
            });
        });

        tui.label("Launch template:");
        tui.ui_add(egui::TextEdit::singleline(&mut settings.launch_template));

        tui.label("Args:");
        tui.ui_add(egui::TextEdit::singleline(&mut settings.launch_params));

        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Row,
            gap: length(8.0),
            flex_shrink: 0.0,
            margin: taffy::Rect {
                left: length(0.0),
                right: length(0.0),
                top: length(8.0),
                bottom: length(0.0),
            },
            size: taffy::Size {
                width: percent(1.),
                height: auto(),
            },
            ..Default::default()
        })
        .add(|tui| {
            if tui
                .ui(|ui| cmd_button(ui, "SAVE", "primary", vm.can_change_network))
                .clicked()
            {
                save_settings = Some(settings.clone());
            }
            if tui
                .ui(|ui| cmd_button(ui, "CANCEL", "outline", true))
                .clicked()
            {
                cancel_clicked = true;
            }
        });
        }

        if let Some(err) = &app.state.pipeline.error {
            tui.ui(|ui| ui.colored_label(egui::Color32::LIGHT_RED, err));
        }

        if cancel_clicked {
            app.state.settings_draft = None;
            app.navigate(Route::ProfileHub);
        } else if let Some(s) = save_settings.take() {
            if app.update_settings(s).is_ok() {
                app.state.settings_draft = None;
                app.navigate(Route::ProfileHub);
            }
        }
    });
}
