use super::PlotWindow;
use iced::Theme;
use plotters::prelude::RGBColor;

pub(super) fn state_to_theme(state: &PlotWindow) -> Theme {
    return mode_to_theme(&state.theme_mode);
}

pub(super) fn mode_to_theme(theme_mode: &iced::theme::Mode) -> Theme {
    match theme_mode {
        iced::theme::Mode::Dark => {
            return Theme::TokyoNight;
        }
        iced::theme::Mode::Light | iced::theme::Mode::None => {
            return Theme::TokyoNightLight;
        }
    }
}

pub(super) fn iced_color_to_plotters(iced: iced::Color) -> RGBColor {
    let plotters = {
        RGBColor(
            (iced.r * 255.0) as u8,
            (iced.g * 255.0) as u8,
            (iced.b * 255.0) as u8,
        )
    };
    return plotters;
}
