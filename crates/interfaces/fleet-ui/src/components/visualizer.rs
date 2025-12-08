use crate::theme::*;
use crate::utils::section_label;
use eframe::egui;
use egui_taffy::taffy::prelude::{length, percent};
use egui_taffy::{taffy, TuiBuilderLogic};
use fleet_app_core::viewmodel::{DashboardState, VisualizerPhase, VisualizerVm};
use std::collections::HashSet;

pub struct Visualizer;

impl Visualizer {
    pub fn draw<'a>(tui: impl TuiBuilderLogic<'a>, state: &DashboardState, vm: &VisualizerVm) {
        tui.style(taffy::Style {
            flex_direction: taffy::FlexDirection::Column,
            gap: length(4.0),
            flex_grow: 1.0,
            size: percent(1.),
            ..Default::default()
        })
        .add(|tui| {
            tui.ui(|ui| section_label(ui, "MANIFEST"));

            tui.style(taffy::Style {
                flex_grow: 1.0,
                flex_shrink: 1.0,
                min_size: taffy::Size {
                    width: length(0.0),
                    height: length(140.0),
                },
                size: percent(1.),
                ..Default::default()
            })
            .ui(|ui| {
                let rect = ui.max_rect();

                ui.painter().rect_filled(rect, 0.0, COL_BG_DARK);
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, COL_BORDER),
                    egui::StrokeKind::Outside,
                );

                let cell_size = 6.0;
                let cell_gap = 1.0;
                let total_cell = cell_size + cell_gap;

                let cols = (rect.width() / total_cell).floor() as usize;
                let rows = (rect.height() / total_cell).floor() as usize;
                let capacity = cols.saturating_mul(rows);
                if capacity == 0 {
                    return;
                }

                #[derive(Clone, Copy, PartialEq, Eq)]
                enum Overlay {
                    Delete,
                    Add,
                    Edit,
                }

                fn fnv1a_64(s: &str) -> u64 {
                    let mut h: u64 = 0xcbf29ce484222325;
                    for b in s.as_bytes() {
                        h ^= *b as u64;
                        h = h.wrapping_mul(0x100000001b3);
                    }
                    h
                }

                fn bucket_idx(key: &str, capacity: usize) -> usize {
                    if capacity == 0 {
                        return 0;
                    }
                    (fnv1a_64(key) as usize) % capacity
                }

                let existing_mods: HashSet<&str> =
                    vm.existing_mods.iter().map(|s| s.as_str()).collect();

                let mut overlays: Vec<Option<Overlay>> = vec![None; capacity];
                if let Some(plan) = &vm.plan {
                    for del in &plan.deletes {
                        let idx = bucket_idx(&del.path, capacity);
                        overlays[idx] = Some(Overlay::Delete);
                    }
                    for dl in &plan.downloads {
                        let key = format!("{}/{}", dl.mod_name, dl.rel_path);
                        let idx = bucket_idx(&key, capacity);
                        if overlays[idx] == Some(Overlay::Delete) {
                            continue;
                        }
                        let overlay = if existing_mods.contains(dl.mod_name.as_str()) {
                            Overlay::Edit
                        } else {
                            Overlay::Add
                        };
                        overlays[idx] = Some(overlay);
                    }
                }

                let mut in_flight_flags = vec![false; capacity];
                if let Some(tp) = &vm.transfer {
                    for f in &tp.active_files {
                        let key = format!("{}/{}", f.mod_name, f.rel_path);
                        let idx = bucket_idx(&key, capacity);
                        if idx < in_flight_flags.len() {
                            in_flight_flags[idx] = true;
                        }
                    }
                }

                let (scan_processed, scan_active) = if let Some(st) = &vm.scan {
                    if st.total_files > 0 {
                        let ratio =
                            (st.files_scanned as f32 / st.total_files as f32).clamp(0.0, 1.0);
                        let processed = (ratio * capacity as f32).floor() as usize;
                        let active = processed.min(capacity.saturating_sub(1));
                        (processed.min(capacity), active)
                    } else if st.files_scanned > 0 {
                        let cursor = st.files_scanned as usize;
                        let processed = (cursor % (capacity + 1)).min(capacity);
                        let active = processed.saturating_sub(1);
                        (processed, active)
                    } else {
                        (0, 0)
                    }
                } else {
                    (0, 0)
                };

                let exec_ratio = vm
                    .transfer
                    .as_ref()
                    .and_then(|tp| {
                        (tp.total_bytes > 0)
                            .then_some(tp.downloaded_bytes as f32 / tp.total_bytes as f32)
                    })
                    .unwrap_or(match state {
                        DashboardState::Synced { .. } => 1.0,
                        _ => 0.0,
                    });

                let exec_processed = (exec_ratio * capacity as f32).floor() as usize;

                let block_status_fn = |idx: usize| -> (egui::Color32, f32, bool) {
                    let is_active_check =
                        vm.phase == VisualizerPhase::Scanning && idx == scan_active;
                    match vm.phase {
                        VisualizerPhase::Scanning => {
                            if idx < scan_processed {
                                (COL_SUCCESS, 0.9, is_active_check)
                            } else if idx == scan_active {
                                (COL_ACCENT, 0.9, is_active_check)
                            } else {
                                (COL_BG, 0.18, is_active_check)
                            }
                        }
                        VisualizerPhase::Executing => {
                            if in_flight_flags.get(idx).copied().unwrap_or(false) {
                                return (COL_ACCENT, 0.95, false);
                            }
                            if idx < exec_processed {
                                (COL_SUCCESS, 0.9, false)
                            } else {
                                (COL_BG, 0.18, false)
                            }
                        }
                        VisualizerPhase::Review => match overlays.get(idx).copied().flatten() {
                            Some(Overlay::Delete) => (COL_DANGER, 0.85, false),
                            Some(Overlay::Add) => (COL_SUCCESS, 0.9, false),
                            Some(Overlay::Edit) => (COL_ACCENT, 0.9, false),
                            None => (COL_SUCCESS, 0.25, false),
                        },
                        VisualizerPhase::Fetching
                        | VisualizerPhase::Diffing
                        | VisualizerPhase::PostScan => {
                            let c = idx % cols;
                            let r = idx / cols;
                            if (r + c).is_multiple_of(23) {
                                (COL_ACCENT, 0.9, false)
                            } else {
                                (COL_BORDER, 0.25, false)
                            }
                        }
                        VisualizerPhase::Synced => (COL_SUCCESS, 0.6, false),
                        VisualizerPhase::Error => (COL_DANGER, 0.4, false),
                        VisualizerPhase::Idle => (COL_BORDER, 0.25, false),
                    }
                };

                let mut mesh = egui::Mesh::default();
                let mut overlay_draws: Vec<(egui::Rect, Option<Overlay>, bool)> =
                    Vec::with_capacity(capacity);
                for (i, overlay) in overlays.iter().enumerate().take(capacity) {
                    let col = i % cols;
                    let row = i / cols;
                    let x = rect.min.x + (col as f32 * total_cell);
                    let y = rect.min.y + (row as f32 * total_cell);
                    let (color, alpha, active_check) = block_status_fn(i);
                    let final_color = color.linear_multiply(alpha);
                    let block_rect = egui::Rect::from_min_size(
                        egui::pos2(x, y),
                        egui::vec2(cell_size, cell_size),
                    );
                    mesh.add_colored_rect(block_rect, final_color);
                    overlay_draws.push((block_rect, *overlay, active_check));
                }
                ui.painter().add(mesh);

                let show_overlays = matches!(
                    vm.phase,
                    VisualizerPhase::Review | VisualizerPhase::Executing
                );

                for (block_rect, overlay, active_check) in overlay_draws {
                    if active_check {
                        ui.painter().rect_stroke(
                            block_rect.shrink(1.0),
                            0.0,
                            egui::Stroke::new(1.0, egui::Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }

                    if !show_overlays {
                        continue;
                    }

                    match overlay {
                        Some(Overlay::Delete) => {
                            ui.painter().line_segment(
                                [block_rect.left_top(), block_rect.right_bottom()],
                                egui::Stroke::new(1.0, COL_DANGER),
                            );
                        }
                        Some(Overlay::Add) => {
                            let center = block_rect.center();
                            let s = block_rect.width().min(block_rect.height()) * 0.35;
                            ui.painter().line_segment(
                                [
                                    egui::pos2(center.x - s, center.y),
                                    egui::pos2(center.x + s, center.y),
                                ],
                                egui::Stroke::new(1.0, COL_SUCCESS),
                            );
                            ui.painter().line_segment(
                                [
                                    egui::pos2(center.x, center.y - s),
                                    egui::pos2(center.x, center.y + s),
                                ],
                                egui::Stroke::new(1.0, COL_SUCCESS),
                            );
                        }
                        Some(Overlay::Edit) => {
                            let r = block_rect.shrink(1.0);
                            let x1 = r.min.x + r.width() * 0.35;
                            let x2 = r.min.x + r.width() * 0.65;
                            let y1 = r.min.y + r.height() * 0.35;
                            let y2 = r.min.y + r.height() * 0.65;
                            ui.painter().line_segment(
                                [egui::pos2(x1, r.min.y), egui::pos2(x1, r.max.y)],
                                egui::Stroke::new(1.0, COL_ACCENT),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(x2, r.min.y), egui::pos2(x2, r.max.y)],
                                egui::Stroke::new(1.0, COL_ACCENT),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(r.min.x, y1), egui::pos2(r.max.x, y1)],
                                egui::Stroke::new(1.0, COL_ACCENT),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(r.min.x, y2), egui::pos2(r.max.x, y2)],
                                egui::Stroke::new(1.0, COL_ACCENT),
                            );
                        }
                        None => {}
                    }
                }
            });
        });
    }
}
