use crate::components::{command, readout, visualizer};
use crate::theme::*;
use crate::utils::cmd_button;
use eframe::egui;
use egui_taffy::taffy::prelude::{length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::viewmodel::ProfileDashboardVm;
use fleet_app_core::FleetApplication;

pub fn draw<'a>(
    tui: impl TuiBuilderLogic<'a>,
    vm: &ProfileDashboardVm,
    app: &mut FleetApplication,
) {
    tui.style(taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        gap: length(8.0),
        size: percent(1.),
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        },
        // Allow dashboard to shrink with the main content column; keep
        // `min_size.width` at 0 so it doesn't force extra width.
        min_size: taffy::Size {
            width: length(0.0),
            height: length(0.0),
        },
        ..Default::default()
    })
    .add(|tui| {
        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Column,
            gap: length(4.0),
            ..Default::default()
        })
        .add(|tui| {
            tui.style(taffy::Style {
                flex_direction: taffy::FlexDirection::Row,
                justify_content: Some(taffy::JustifyContent::SpaceBetween),
                align_items: Some(taffy::AlignItems::Center),
                ..Default::default()
            })
            .add(|tui| {
                tui.label(
                    egui::RichText::new(&vm.profile.name)
                        .size(14.0)
                        .strong()
                        .color(COL_TEXT),
                );

                if tui
                    .ui(|ui| cmd_button(ui, "EDIT", "outline", true))
                    .clicked()
                {
                    app.edit_profile(vm.profile.id.clone());
                }
            });

            tui.label(
                egui::RichText::new(format!("PATH: {}", vm.profile.local_path))
                    .size(10.0)
                    .color(COL_TEXT_DIM),
            );

            tui.separator();
        });

        readout::draw(&mut *tui, &vm.stats);

        let cmd_resp = command::draw(&mut *tui, &vm.state);
        if cmd_resp.check_local {
            if let Err(e) = app.local_check(vm.profile.id.clone()) {
                tracing::error!("Failed to start local check: {e}");
            }
        }
        if cmd_resp.check_remote {
            if let Err(e) = app.check_for_updates(vm.profile.id.clone()) {
                tracing::error!("Failed to start remote check: {e}");
            }
        }
        if cmd_resp.repair {
            if let Err(e) = app.repair(vm.profile.id.clone()) {
                tracing::error!("Failed to repair profile: {e}");
            }
        }
        if cmd_resp.sync {
            if let Err(e) = app.execute_sync(vm.profile.id.clone()) {
                tracing::error!("Failed to start sync: {e}");
            }
        }
        if cmd_resp.launch {
            if let Err(e) = app.launch_profile(vm.profile.id.clone()) {
                tracing::error!("Failed to launch profile: {e}");
            }
        }
        if cmd_resp.join {
            if let Err(e) = app.join_profile(vm.profile.id.clone()) {
                tracing::error!("Failed to join profile: {e}");
            }
        }
        if cmd_resp.cancel {
            app.cancel_pipeline();
        }
        if cmd_resp.ack {
            app.acknowledge_pipeline_completion();
        }

        visualizer::Visualizer::draw(&mut *tui, &vm.state, &vm.visualizer);
    });
}
