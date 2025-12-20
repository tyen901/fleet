use crate::ui_kit::UiKit;
use crate::widgets;
use eframe::egui;
use fleet_app::ProfileSpec;

pub enum SidebarAction {
    NewProfile,
    OpenProfile(String),
    OpenSettings,
    Refresh,
}

pub fn draw(
    ui: &mut egui::Ui,
    kit: &UiKit,
    filter: &mut String,
    profiles: &[ProfileSpec],
    selected_profile_id: Option<&str>,
) -> Option<SidebarAction> {
    let mut out = None;

    ui.set_min_width(ui.available_width());

    ui.vertical(|ui| {
        // Top row: label + actions right-aligned
        ui.horizontal(|ui| {
            ui.add(widgets::FieldLabel::new(kit, "Profiles"));

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(
                            widgets::AppButton::new(kit, "Refresh")
                                .ghost()
                                .min_width(80.0),
                        )
                        .clicked()
                    {
                        out = Some(SidebarAction::Refresh);
                    }

                    if ui
                        .add(
                            widgets::AppButton::new(kit, "+ New")
                                .ghost()
                                .min_width(70.0),
                        )
                        .clicked()
                    {
                        out = Some(SidebarAction::NewProfile);
                    }
                },
            );
        });

        ui.add(widgets::Divider::new(kit));

        ui.add(widgets::FieldLabel::new(kit, "Filter"));
        widgets::text_field(ui, kit, filter, "Type to filter…", false);

        ui.add(widgets::Divider::new(kit));

        let footer_h = kit.layout.button_height + kit.layout.gap + 1.0;
        let list_h = (ui.available_height() - footer_h).max(0.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(list_h)
            .show(ui, |ui| {
                let needle = filter.trim().to_lowercase();

                for p in profiles {
                    if !needle.is_empty()
                        && !p.name.to_lowercase().contains(&needle)
                        && !p.id.to_lowercase().contains(&needle)
                    {
                        continue;
                    }

                    let selected = selected_profile_id.is_some_and(|id| id == p.id);
                    let text = egui::RichText::new(&p.name).size(kit.theme.type_scale.body);

                    let mut resp = ui.add_sized(
                        [ui.available_width(), kit.layout.row_height],
                        egui::Button::selectable(selected, text),
                    );

                    if resp.hovered() {
                        resp = resp.on_hover_text(format!("{}\n{}", p.name, p.id));
                    }
                    if resp.clicked() {
                        out = Some(SidebarAction::OpenProfile(p.id.clone()));
                    }
                }

                if profiles.is_empty() {
                    ui.add_space(kit.layout.gap);
                    ui.add(widgets::InlineHint::new(kit, "No profiles found."));
                }
            });

        ui.add(widgets::Divider::new(kit));

        if ui
            .add(
                widgets::AppButton::new(kit, "Settings")
                    .ghost()
                    .min_width(90.0),
            )
            .clicked()
        {
            out = Some(SidebarAction::OpenSettings);
        }
    });

    out
}
