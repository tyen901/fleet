use eframe::egui::{Color32, CornerRadius};

#[derive(Debug, Clone)]
pub struct Theme {
    pub colors: Colors,
    pub sizes: Sizes,
    pub spacing: Spacing,
}

#[derive(Debug, Clone)]
pub struct Colors {
    // Synk-style tokens
    pub bg_app: Color32,
    pub bg_shell: Color32,
    pub bg_subtle: Color32,
    pub bg_surface: Color32,
    pub bg_surface_hover: Color32,

    pub border: Color32,
    pub border_strong: Color32,

    pub text_main: Color32,
    pub text_muted: Color32,
    pub text_dim: Color32,

    pub brand: Color32,
    pub brand_fg: Color32,

    pub status_success: Color32,
    pub status_warning: Color32,
    pub status_error: Color32,
    pub status_info: Color32,
}

#[derive(Debug, Clone)]
pub struct Sizes {
    pub shell_max_width: f32,
    pub sidebar_width: f32,   // ~w-11
    pub header_height: f32,   // ~h-9
    pub sidebar_cell: f32,    // ~h-11
    pub icon_button: f32,     // ~h-9
    pub button_height: f32,   // ~h-7
    pub list_row_height: f32, // ~h-10
}

#[derive(Debug, Clone)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            colors: Colors {
                bg_app: Color32::from_rgb(0x0c, 0x0c, 0x0c),
                bg_shell: Color32::from_rgb(0x11, 0x11, 0x11),
                bg_subtle: Color32::from_rgb(0x16, 0x16, 0x16),
                bg_surface: Color32::from_rgb(0x1a, 0x1a, 0x1a),
                bg_surface_hover: Color32::from_rgb(0x22, 0x22, 0x22),

                border: Color32::from_rgb(0x2a, 0x2a, 0x2a),
                border_strong: Color32::from_rgb(0x3a, 0x3a, 0x3a),

                text_main: Color32::from_rgb(0xc8, 0xc8, 0xc8),
                text_muted: Color32::from_rgb(0x88, 0x88, 0x88),
                text_dim: Color32::from_rgb(0x55, 0x55, 0x55),

                brand: Color32::from_rgb(0xc8, 0xc8, 0xc8),
                brand_fg: Color32::from_rgb(0x0c, 0x0c, 0x0c),

                status_success: Color32::from_rgb(0x77, 0xdd, 0x77),
                status_warning: Color32::from_rgb(0xff, 0xb3, 0x47),
                status_error: Color32::from_rgb(0xff, 0x69, 0x61),
                status_info: Color32::from_rgb(0x77, 0xb5, 0xfe),
            },
            sizes: Sizes {
                shell_max_width: 1100.0,
                sidebar_width: 44.0,
                header_height: 36.0,
                sidebar_cell: 44.0,
                icon_button: 36.0,
                button_height: 28.0,
                list_row_height: 40.0,
            },
            spacing: Spacing {
                xs: 4.0,
                sm: 8.0,
                md: 12.0,
                lg: 16.0,
                xl: 24.0,
            },
        }
    }

    pub fn light() -> Self {
        Self {
            colors: Colors {
                bg_app: Color32::from_rgb(0xe8, 0xe8, 0xe8),
                bg_shell: Color32::from_rgb(0xf0, 0xf0, 0xf0),
                bg_subtle: Color32::from_rgb(0xe0, 0xe0, 0xe0),
                bg_surface: Color32::from_rgb(0xd8, 0xd8, 0xd8),
                bg_surface_hover: Color32::from_rgb(0xd0, 0xd0, 0xd0),

                border: Color32::from_rgb(0xc0, 0xc0, 0xc0),
                border_strong: Color32::from_rgb(0xa0, 0xa0, 0xa0),

                text_main: Color32::from_rgb(0x1a, 0x1a, 0x1a),
                text_muted: Color32::from_rgb(0x50, 0x50, 0x50),
                text_dim: Color32::from_rgb(0x80, 0x80, 0x80),

                brand: Color32::from_rgb(0x1a, 0x1a, 0x1a),
                brand_fg: Color32::from_rgb(0xf0, 0xf0, 0xf0),

                status_success: Color32::from_rgb(0x1f, 0x8b, 0x4c),
                status_warning: Color32::from_rgb(0xb7, 0x7b, 0x00),
                status_error: Color32::from_rgb(0xb0, 0x00, 0x20),
                status_info: Color32::from_rgb(0x00, 0x4f, 0xbf),
            },
            sizes: Sizes {
                shell_max_width: 1100.0,
                sidebar_width: 44.0,
                header_height: 36.0,
                sidebar_cell: 44.0,
                icon_button: 36.0,
                button_height: 28.0,
                list_row_height: 40.0,
            },
            spacing: Spacing {
                xs: 4.0,
                sm: 8.0,
                md: 12.0,
                lg: 16.0,
                xl: 24.0,
            },
        }
    }

    pub fn square_rounding() -> CornerRadius {
        CornerRadius::same(0)
    }
}
