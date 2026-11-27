use super::Message;
use crate::store::{MessageSend, TableStore};
use iced::widget::{Button, Column, Row, container, rule, text, text_input};
use iced::{Element, Length, Padding, Theme, alignment};
use std::collections::HashMap;

pub(super) struct MessageSendTable {
    message: dbc_rs::Message,
    store: TableStore,
    theme: iced::Theme,
    edits: HashMap<String, String>, // signal name -> current text
}

impl<'a> MessageSendTable {
    pub fn new(
        message: dbc_rs::Message,
        store: TableStore,
        theme: iced::Theme,
        edits: HashMap<String, String>,
    ) -> Self {
        Self {
            message,
            store,
            theme,
            edits,
        }
    }

    pub fn view(&self) -> Element<'a, Message> {
        const ROW_H: f32 = 24.0;
        const INPUT_W: f32 = 80.0;

        let text_color = self.theme.palette().text;
        let rule_color = self.theme.extended_palette().background.strong.color;
        let msg_name = self.message.name().to_string();

        let mut names = Column::new().spacing(4);
        let mut values = Column::new()
            .spacing(4)
            .align_x(alignment::Horizontal::Right);
        let mut content: Vec<(Box<str>, f64)> = Vec::new();

        let signal_list = self.message.signals();
        let last = signal_list.len().saturating_sub(1);

        for (i, signal) in signal_list.iter().enumerate() {
            let sig_name = signal.name().to_string();

            // Default comes from the store, exactly as before
            let default = self
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

            // Text shown: the user's edit if there is one, else the default
            let shown = self
                .edits
                .get(&sig_name)
                .cloned()
                .unwrap_or_else(|| format!("{:.2}", default));

            // Value to send: parsed text, falling back to the default if it doesn't parse
            let parsed = shown.trim().parse::<f64>().unwrap_or(default);
            content.push((sig_name.clone().into_boxed_str(), parsed));

            let (m, s) = (msg_name.clone(), sig_name.clone());
            let input = text_input("0.00", &shown)
                .size(12)
                .padding(3)
                .width(Length::Fixed(INPUT_W))
                .align_x(alignment::Horizontal::Right)
                .on_input(move |v| Message::SignalEdited {
                    message: m.clone(),
                    signal: s.clone(),
                    value: v,
                });

            // Both cells share a fixed height so the two columns stay aligned
            names = names.push(
                container(text(sig_name).size(12).color(text_color))
                    .height(Length::Fixed(ROW_H))
                    .align_y(alignment::Vertical::Center)
                    .padding(Padding {
                        top: 0.0,
                        right: 16.0,
                        bottom: 0.0,
                        left: 0.0,
                    }),
            );
            values = values.push(
                container(input)
                    .height(Length::Fixed(ROW_H))
                    .align_y(alignment::Vertical::Center),
            );

            if i != last {
                let style = move |_t: &Theme| rule::Style {
                    color: rule_color,
                    radius: 0.0.into(),
                    fill_mode: rule::FillMode::Full,
                    snap: true,
                };
                names = names.push(rule::horizontal(1).style(style));
                values = values.push(rule::horizontal(1).style(style));
            }
        }

        let title = Row::new()
            .push(text(msg_name.clone()))
            .push(iced::widget::Space::new().width(Length::Fill))
            .push(
                Button::new("Send").on_press(Message::SendFrame(MessageSend {
                    name: msg_name,
                    content,
                    repeat: 1,
                })),
            )
            .spacing(16)
            .align_y(alignment::Vertical::Bottom)
            .width(Length::Shrink);

        let signals = Row::new().push(names).push(values);
        let indented = container(signals).width(Length::Shrink).padding(Padding {
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
            .width(Length::Shrink)
            .height(Length::Shrink)
            .into()
    }
}
