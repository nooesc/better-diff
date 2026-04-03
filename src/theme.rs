use std::sync::OnceLock;

use ratatui::style::Color;

static ACTIVE_THEME: OnceLock<Theme> = OnceLock::new();

pub fn init(name: &str) {
    let theme = match name {
        "light" => Theme::light(),
        "monokai" => Theme::monokai(),
        _ => Theme::dark(),
    };
    let _ = ACTIVE_THEME.set(theme);
}

pub fn current() -> &'static Theme {
    ACTIVE_THEME.get_or_init(Theme::dark)
}

pub struct Theme {
    // Syntax highlighting
    pub syntax_comment: Color,
    pub syntax_string: Color,
    pub syntax_number: Color,
    pub syntax_type: Color,
    pub syntax_keyword: Color,
    pub syntax_function: Color,
    pub syntax_macro: Color,

    // Diff colors
    pub diff_add_fg: Color,
    pub diff_add_bg: Color,
    pub diff_add_token_bg: Color,
    pub diff_del_fg: Color,
    pub diff_del_bg: Color,
    pub diff_del_token_bg: Color,
    pub diff_rename_fg: Color,
    pub diff_rename_bg: Color,

    // UI colors
    pub ui_line_number: Color,
    pub ui_header: Color,
    pub ui_fold: Color,
    pub ui_move: Color,
    pub ui_tab_active: Color,
    pub ui_tab_inactive: Color,
    pub ui_mode: Color,
    pub ui_key_hint: Color,
    pub ui_dim: Color,
    pub ui_live: Color,
    pub ui_search_current_bg: Color,
    pub ui_search_current_fg: Color,
    pub ui_search_other_bg: Color,
    pub ui_search_other_fg: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            syntax_comment: Color::DarkGray,
            syntax_string: Color::Rgb(206, 145, 120),
            syntax_number: Color::Rgb(181, 206, 168),
            syntax_type: Color::Rgb(78, 201, 176),
            syntax_keyword: Color::Rgb(197, 134, 192),
            syntax_function: Color::Rgb(220, 220, 170),
            syntax_macro: Color::Rgb(86, 156, 214),

            diff_add_fg: Color::Green,
            diff_add_bg: Color::Rgb(0, 40, 0),
            diff_add_token_bg: Color::Rgb(0, 80, 0),
            diff_del_fg: Color::Red,
            diff_del_bg: Color::Rgb(40, 0, 0),
            diff_del_token_bg: Color::Rgb(80, 0, 0),
            diff_rename_fg: Color::Blue,
            diff_rename_bg: Color::Rgb(0, 0, 80),

            ui_line_number: Color::DarkGray,
            ui_header: Color::DarkGray,
            ui_fold: Color::DarkGray,
            ui_move: Color::Magenta,
            ui_tab_active: Color::Cyan,
            ui_tab_inactive: Color::DarkGray,
            ui_mode: Color::Cyan,
            ui_key_hint: Color::Yellow,
            ui_dim: Color::DarkGray,
            ui_live: Color::Green,
            ui_search_current_bg: Color::Yellow,
            ui_search_current_fg: Color::Black,
            ui_search_other_bg: Color::DarkGray,
            ui_search_other_fg: Color::White,
        }
    }

    pub fn light() -> Self {
        Self {
            syntax_comment: Color::Rgb(128, 128, 128),
            syntax_string: Color::Rgb(163, 21, 21),
            syntax_number: Color::Rgb(9, 134, 88),
            syntax_type: Color::Rgb(38, 127, 153),
            syntax_keyword: Color::Rgb(0, 0, 255),
            syntax_function: Color::Rgb(121, 94, 38),
            syntax_macro: Color::Rgb(0, 112, 193),

            diff_add_fg: Color::Rgb(0, 100, 0),
            diff_add_bg: Color::Rgb(220, 255, 220),
            diff_add_token_bg: Color::Rgb(180, 255, 180),
            diff_del_fg: Color::Rgb(160, 0, 0),
            diff_del_bg: Color::Rgb(255, 220, 220),
            diff_del_token_bg: Color::Rgb(255, 180, 180),
            diff_rename_fg: Color::Rgb(0, 0, 180),
            diff_rename_bg: Color::Rgb(220, 220, 255),

            ui_line_number: Color::Rgb(128, 128, 128),
            ui_header: Color::Rgb(128, 128, 128),
            ui_fold: Color::Rgb(128, 128, 128),
            ui_move: Color::Rgb(128, 0, 128),
            ui_tab_active: Color::Rgb(0, 100, 150),
            ui_tab_inactive: Color::Rgb(128, 128, 128),
            ui_mode: Color::Rgb(0, 100, 150),
            ui_key_hint: Color::Rgb(150, 100, 0),
            ui_dim: Color::Rgb(128, 128, 128),
            ui_live: Color::Rgb(0, 128, 0),
            ui_search_current_bg: Color::Rgb(255, 200, 0),
            ui_search_current_fg: Color::Black,
            ui_search_other_bg: Color::Rgb(200, 200, 200),
            ui_search_other_fg: Color::Black,
        }
    }

    pub fn monokai() -> Self {
        Self {
            syntax_comment: Color::Rgb(117, 113, 94),
            syntax_string: Color::Rgb(230, 219, 116),
            syntax_number: Color::Rgb(174, 129, 255),
            syntax_type: Color::Rgb(102, 217, 239),
            syntax_keyword: Color::Rgb(249, 38, 114),
            syntax_function: Color::Rgb(166, 226, 46),
            syntax_macro: Color::Rgb(102, 217, 239),

            diff_add_fg: Color::Rgb(166, 226, 46),
            diff_add_bg: Color::Rgb(20, 40, 10),
            diff_add_token_bg: Color::Rgb(40, 70, 20),
            diff_del_fg: Color::Rgb(249, 38, 114),
            diff_del_bg: Color::Rgb(50, 10, 20),
            diff_del_token_bg: Color::Rgb(80, 20, 30),
            diff_rename_fg: Color::Rgb(102, 217, 239),
            diff_rename_bg: Color::Rgb(10, 30, 50),

            ui_line_number: Color::Rgb(117, 113, 94),
            ui_header: Color::Rgb(117, 113, 94),
            ui_fold: Color::Rgb(117, 113, 94),
            ui_move: Color::Rgb(174, 129, 255),
            ui_tab_active: Color::Rgb(102, 217, 239),
            ui_tab_inactive: Color::Rgb(117, 113, 94),
            ui_mode: Color::Rgb(102, 217, 239),
            ui_key_hint: Color::Rgb(230, 219, 116),
            ui_dim: Color::Rgb(117, 113, 94),
            ui_live: Color::Rgb(166, 226, 46),
            ui_search_current_bg: Color::Rgb(230, 219, 116),
            ui_search_current_fg: Color::Black,
            ui_search_other_bg: Color::Rgb(60, 60, 40),
            ui_search_other_fg: Color::White,
        }
    }
}
