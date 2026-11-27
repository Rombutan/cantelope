use super::Message;
use crate::store::TableStore;
use iced::widget::{Button, Column, Row, container, rule, text};
use iced::{Element, Length, Padding, Theme, alignment};
use iced_aw::{helpers::card, style};

pub(super) struct NodeTable {
    // The specific signal name to look up
    name: String,
    store: TableStore,
    theme: iced::Theme,
}

impl<'a> NodeTable {
    pub fn new(name: String, store: TableStore, theme: iced::Theme) -> Self {
        Self {
            name: name,
            store: store,
            theme: theme,
        }
    }
    pub fn view(&self) -> Element<'a, Message> {
        // Node name, larger than the 12pt message text
        let title = text(self.name.clone())
            .size(16)
            .color(self.theme.palette().text);

        // Clone the node's messages out so the read lock is released right away
        let node_messages: Vec<dbc_rs::Message> = self
            .store
            .nodes
            .read()
            .get(self.name.as_str())
            .cloned()
            .unwrap_or_default();

        let messages = node_messages
            .into_iter()
            .fold(Column::new().spacing(4), |col, msg| {
                col.push(MessageTable::new(msg, self.store.clone_ref(), self.theme.clone()).view())
            })
            .width(Length::Shrink); // after the fold, not before

        let palette = self.theme.extended_palette();
        let body_bg = palette.background.base.color;
        let body_text = palette.background.base.text;
        let head_bg = palette.background.strong.color;
        let head_text = palette.background.strong.text;

        let card_style = move |theme: &Theme, status: iced_aw::card::Status| style::card::Style {
            background: body_bg.into(),
            border_color: head_bg,
            head_background: head_bg.into(),
            head_text_color: head_text,
            body_background: body_bg.into(),
            body_text_color: body_text,
            ..style::card::dark(theme, status)
        };

        let element: Element<'_, Message> = card(title, messages)
            .style(card_style)
            .padding_head(Padding::new(6.0)) // head padding
            .padding_body(Padding::new(6.0)) // replaces your manual indent container
            .width(Length::Fill) // fills the fixed-width cell from the top-level layout
            .height(Length::Shrink)
            .into();

        element
    }
}

struct MessageTable {
    message: dbc_rs::Message,
    store: TableStore,
    theme: iced::Theme,
}

impl<'a> MessageTable {
    pub fn new(message: dbc_rs::Message, store: TableStore, theme: iced::Theme) -> Self {
        Self {
            message: message,
            store: store,
            theme: theme,
        }
    }
    pub fn view(&self) -> Element<'a, Message> {
        let text_color = self.theme.palette().text;

        let title = Row::new()
            .push(text(self.message.name().to_string()))
            .push(iced::widget::Space::new().width(Length::Fill))
            .push(Button::new("Send").on_press(Message::SetSend(self.message.name().to_string())))
            .spacing(16)
            .align_y(alignment::Vertical::Bottom)
            .width(Length::Shrink);

        let mut names = Column::new().spacing(4);
        let mut values = Column::new()
            .spacing(4)
            .align_x(alignment::Horizontal::Right);

        let signal_list = self.message.signals();
        let last = signal_list.len().saturating_sub(1);

        for (i, signal) in signal_list.iter().enumerate() {
            let value = self
                .store
                .get_latest_valid_value_by_index(
                    self.store
                        .signalmap
                        .read()
                        .get(signal.name())
                        .unwrap()
                        .clone(),
                )
                .unwrap_or(0.0f64);

            // Gap lives inside the name cell, so the rule below spans it
            names = names.push(
                container(text(signal.name().to_string()).size(12).color(text_color)).padding(
                    Padding {
                        top: 0.0,
                        right: 16.0,
                        bottom: 0.0,
                        left: 0.0,
                    },
                ),
            );
            values = values.push(text(format!("{:.2}", value)).size(12).color(text_color));

            let rule_color = self.theme.extended_palette().background.strong.color;
            if i != last {
                // Same rule in both columns so heights stay in sync
                names = names.push(
                    rule::horizontal(1).style(move |_theme: &Theme| rule::Style {
                        color: rule_color,
                        radius: 0.0.into(),
                        fill_mode: rule::FillMode::Full,
                        snap: true,
                    }),
                );
                values =
                    values.push(
                        rule::horizontal(1).style(move |_theme: &Theme| rule::Style {
                            color: rule_color,
                            radius: 0.0.into(),
                            fill_mode: rule::FillMode::Full,
                            snap: true,
                        }),
                    );
            }
        }

        let signals = Row::new().push(names).push(values);

        let indented = container(signals)
            .width(Length::Shrink) // after container::new
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 20.0,
            });

        Column::new()
            .spacing(5)
            .push(title)
            .push(indented)
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 5.0,
            })
            .width(Length::Shrink) // replaces Length::Fill
            .height(Length::Shrink) // replaces Length::Fill
            .into()
    }
}
