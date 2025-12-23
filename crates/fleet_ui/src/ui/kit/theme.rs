use egui;

#[derive(Clone, Debug)]
pub struct Colors {
    pub text: egui::Color32,
    pub muted: egui::Color32,
    pub accent: egui::Color32,
    pub danger: egui::Color32,
    pub border: egui::Color32,
    pub panel: egui::Color32,
    pub panel_alt: egui::Color32,
}

#[derive(Clone, Debug)]
pub struct TypeScale {
    pub h1: f32,
    pub body: f32,
    pub mono: f32,
}

#[derive(Clone, Debug)]
pub struct Spacing {
    pub sm: f32,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub colors: Colors,
    pub type_scale: TypeScale,
    pub spacing: Spacing,
    pub rounding: Rounding,
}

#[derive(Clone, Debug)]
pub struct Rounding {
    pub card: f32,
}

impl Default for Theme {
    fn default() -> Self {
        let dark = true;
        if dark {
            Self {
                colors: Colors {
                    text: egui::Color32::from_rgb(230, 230, 230),
                    muted: egui::Color32::from_rgb(150, 155, 165),
                    accent: egui::Color32::from_rgb(110, 168, 255),
                    danger: egui::Color32::from_rgb(255, 115, 115),
                    border: egui::Color32::from_rgb(55, 60, 70),
                    panel: egui::Color32::from_rgb(25, 28, 34),
                    panel_alt: egui::Color32::from_rgb(20, 22, 27),
                },
                type_scale: TypeScale {
                    h1: 22.0,
                    body: 14.0,
                    mono: 12.0,
                },
                spacing: Spacing { sm: 8.0 },
                rounding: Rounding { card: 6.0 },
            }
        } else {
            Self {
                colors: Colors {
                    text: egui::Color32::from_rgb(20, 20, 20),
                    muted: egui::Color32::from_rgb(90, 95, 105),
                    accent: egui::Color32::from_rgb(40, 110, 200),
                    danger: egui::Color32::from_rgb(190, 45, 45),
                    border: egui::Color32::from_rgb(210, 215, 225),
                    panel: egui::Color32::from_rgb(250, 250, 252),
                    panel_alt: egui::Color32::from_rgb(245, 246, 249),
                },
                type_scale: TypeScale {
                    h1: 22.0,
                    body: 14.0,
                    mono: 12.0,
                },
                spacing: Spacing { sm: 8.0 },
                rounding: Rounding { card: 6.0 },
            }
        }
    }
}
