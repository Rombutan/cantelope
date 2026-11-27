use super::Message;
use crate::store;
use iced::widget::{container, text};
use iced::{Background, Element};

pub(super) struct SignalStatus {
    // The specific signal name to look up
    name: String,
    // true for >, false for <
    is_greater: bool,
    threshold: f64,
    // The shared signal data
    store: store::TableStore,
}

impl<'a> SignalStatus {
    pub fn new(tuple: (String, bool, f64), store: store::TableStore) -> Self {
        Self {
            name: tuple.0,
            is_greater: tuple.1,
            threshold: tuple.2,
            store: store,
        }
    }

    pub fn view(&self) -> Element<'a, Message> {
        let latest_val = self
            .store
            .get_latest_valid_value_by_index(
                self.store.signalmap.read().get(&self.name).unwrap().clone(),
            )
            .unwrap_or(0.0);

        let is_ok = if self.is_greater {
            latest_val > self.threshold
        } else {
            latest_val < self.threshold
        };

        // Explicitly define the color as a concrete iced::Color
        let box_color = if is_ok {
            iced::Color::from_rgba(0.0, 0.5, 0.0, 0.8) // Green
        } else {
            iced::Color::from_rgba(0.5, 0.0, 0.0, 0.8) // Red
        };

        let operator_str = if self.is_greater { ">" } else { "<" };

        container(
            text(format!(
                "{}\n{} {:.1}  |  Value: {:.2}",
                self.name, operator_str, self.threshold, latest_val
            ))
            .size(12)
            .color(iced::Color::WHITE),
        )
        .padding(10)
        // Corrected Styling Logic
        .style(move |theme| {
            let mut style = container::rounded_box(theme).background(Background::Color(box_color));
            style.text_color = Some(theme.palette().text);
            style
        })
        .into()
    }
}
