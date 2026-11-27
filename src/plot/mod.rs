use iced::Alignment::Center;
use iced::{
    Element, Length, Size, Subscription, alignment, keyboard, time, widget::Button, widget::Column,
    widget::Row, widget::Text, widget::responsive, widget::scrollable, widget::text_input,
    window::icon::from_file_data,
};

use iced::widget::container;
use plotters_iced2::ChartWidget;
use std::collections::HashMap;
use std::time::Instant;
use std::{f64, usize};

use crate::store::{self, MessageSend};

use crate::args::{self, CanDataInput};
use args::Args;

mod chart;
mod send;
mod status;
mod tables;
mod theme;
use chart::SignalChart;
use send::MessageSendTable;
use status::SignalStatus;
use tables::NodeTable;
use theme::{mode_to_theme, state_to_theme};

pub const WINDOW_TITLE: &str = concat!("Cantelope  -  ", env!("GIT_HASH"));

const MIN_PLOT_HEIGHT: f32 = 150.0; // smallest a plot is allowed to get
const PLOT_SPACING: f32 = 5.0;

#[derive(Debug, Clone)]
pub enum Tab {
    Plot,
    Signals,
    Send,
}

pub struct PlotWindow {
    store: store::TableStore,
    last_redraw: Instant,
    args: Args,
    play: bool,
    time_range_text: String,
    x_window: f64,
    x_end: f64,
    x_live_attach: bool,
    theme_mode: iced::theme::Mode,
    theme: iced::Theme,
    current_tab: Tab,
    current_send_msg: String,
    send_inputs: HashMap<(String, String), String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Pause,
    TimeRange(String),
    Live(bool),
    AdjustEndTime(f64),
    BumpTime(bool),
    PlotChanged((usize, String)),
    ThemeChanged(iced::theme::Mode),
    SetTab(Tab),
    SetSend(String),
    SendFrame(MessageSend),
    SignalEdited {
        message: String,
        signal: String,
        value: String,
    },
    SetView {
        start: f64,
        end: f64,
    },
    Nothing,
}

impl PlotWindow {
    pub fn run(store: store::TableStore, _args: Args) -> iced::Result {
        store.wait_until_ready();
        let args = _args.clone();
        iced::application(
            {
                move || {
                    let time_range;
                    if args.candatainput == CanDataInput::Parquet {
                        let cols = store.read_columns();
                        let current_index = cols.first().unwrap().len() - 1;
                        time_range = match cols.first() {
                            Some(store::GenericColumn::F64(c)) if !c.values.is_empty() => {
                                c.values[current_index]
                            }
                            _ => 30000.0,
                        };
                    } else {
                        time_range = 3000.0
                    }
                    (
                        PlotWindow {
                            store: store.clone_ref(),
                            last_redraw: Instant::now(),
                            args: args.clone(),
                            play: true,
                            time_range_text: (time_range as u32).to_string(),
                            x_window: time_range,
                            x_end: 0.0,
                            x_live_attach: true,
                            theme_mode: iced::theme::Mode::Light,
                            current_tab: Tab::Plot,
                            current_send_msg: "None".to_string(),
                            send_inputs: HashMap::new(),
                            theme: mode_to_theme(&iced::theme::Mode::Light),
                        },
                        iced::system::theme().map(Message::ThemeChanged),
                    )
                }
            },
            PlotWindow::update,
            PlotWindow::view,
        )
        .subscription(PlotWindow::subscription)
        .title(WINDOW_TITLE)
        .theme(state_to_theme)
        .window(iced::window::Settings {
            size: Size::new(1400.0, 1000.0),
            icon: from_file_data(include_bytes!("../../icon.png"), None).ok(),
            ..Default::default()
        })
        .centered()
        .run()
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick => {
                if self.play {
                    self.last_redraw = Instant::now();
                }
            }
            Message::TimeRange(value) => {
                self.time_range_text = value;
                if let Ok(x_w) = self.time_range_text.parse() {
                    self.x_window = x_w;
                }
            }
            Message::ThemeChanged(mode) => {
                self.theme_mode = mode;
                self.theme = mode_to_theme(&mode);
            }
            Message::BumpTime(up) => {
                if up {
                    self.x_window = self.x_window + 1000.0;
                } else if self.x_window > 1100.0 {
                    // Minimum time is 100 msec I guess
                    self.x_window = self.x_window - 1000.0;
                } else if self.x_window > 200.0 {
                    self.x_window = self.x_window - 100.0;
                }
                self.time_range_text = self.x_window.to_string();
            }
            Message::Live(live) => {
                if self.x_live_attach {
                    if !live {
                        self.x_live_attach = false;
                    }
                } else {
                    if live {
                        self.x_live_attach = true;
                    }
                }
            }
            Message::AdjustEndTime(adjustment) => {
                if !self.x_live_attach {
                    self.x_end += adjustment;
                } else {
                    if adjustment < 0.1 {
                        self.x_live_attach = false;
                        self.x_end = self.store.get_latest_valid_value_by_index(0).unwrap_or(0.0)
                            + adjustment;
                    }
                }
                if self.x_end >= self.store.get_latest_valid_value_by_index(0).unwrap_or(0.0) {
                    self.x_end = self.store.get_latest_valid_value_by_index(0).unwrap_or(0.0);
                }
            }
            Message::SetTab(tab) => {
                self.current_tab = tab;
            }
            Message::SetSend(frame) => {
                self.current_send_msg = frame;
                self.current_tab = Tab::Send;
            }
            Message::SendFrame(frame) => {
                let mut sends = self.store.sends.write();
                sends.push(frame);
            }
            Message::SignalEdited {
                message,
                signal,
                value,
            } => {
                self.send_inputs.insert((message, signal), value);
            }
            Message::SetView { start, end } => {
                let latest = self.store.get_latest_valid_value_by_index(0).unwrap_or(0.0);
                self.x_window = (end - start).max(10.0); // 10 ms minimum
                self.time_range_text = format!("{:.0}", self.x_window);
                if end >= latest - 1e-3 {
                    self.x_end = latest;
                } else {
                    self.x_end = end;
                    self.x_live_attach = false;
                }
            }
            Message::Nothing => {}
            _ => {}
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let timer = time::every(std::time::Duration::from_millis(1000 / 30)).map(|_| Message::Tick);

        let keyboard = keyboard::listen().map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                use keyboard::key::Named;

                // Horizontal panning only: ctrl = 10x, ctrl+shift = 100x
                let h_mult = if modifiers.control() && modifiers.shift() {
                    100.0
                } else if modifiers.control() {
                    10.0
                } else {
                    1.0
                };

                match key {
                    keyboard::Key::Named(Named::Space) | keyboard::Key::Named(Named::Pause) => {
                        Message::Pause
                    }
                    keyboard::Key::Named(Named::ArrowUp) => Message::BumpTime(true),
                    keyboard::Key::Named(Named::ArrowDown) => Message::BumpTime(false),
                    keyboard::Key::Named(Named::ArrowLeft) => {
                        Message::AdjustEndTime(-100.0 * h_mult)
                    }
                    keyboard::Key::Named(Named::ArrowRight) => {
                        Message::AdjustEndTime(100.0 * h_mult)
                    }
                    keyboard::Key::Named(Named::Enter) => Message::Live(true),
                    _ => Message::Nothing,
                }
            }
            _ => Message::Nothing,
        });

        Subscription::batch(vec![timer, keyboard])
    }

    fn view(&self) -> Element<'_, Message> {
        use iced::widget::row;

        let mut controls = Row::new()
            .spacing(10)
            .padding(10)
            .push(Button::new("Plots").on_press(Message::SetTab(Tab::Plot)));

        if self.args.candatainput != CanDataInput::Parquet {
            controls =
                controls.push(Button::new("Signals").on_press(Message::SetTab(Tab::Signals)));
        }

        controls = controls
            .push(Text::new(format!("Period(ms):")).align_y(Center))
            .push(
                text_input("3000", &self.time_range_text)
                    .on_input(Message::TimeRange)
                    .width(100),
            )
            .push(Text::new(format!("File: {}", self.args.input.clone())).align_y(Center));

        let status_row =
            self.args
                .regrens
                .iter()
                .fold(row![].spacing(10).padding(10), |row, tuple| {
                    // .clone() the tuple (or the strings inside it)
                    // so SignalStatus owns the data, not a reference to the lock
                    row.push(SignalStatus::new(tuple.clone(), self.store.clone_ref()).view())
                });

        let plots = responsive(move |size| {
            let n = self.args.plots.len().max(1) as f32;

            // Height each plot would get if they all shared the space evenly
            let fit_height = (size.height - PLOT_SPACING * (n - 1.0)) / n;
            let plot_height = fit_height.floor().max(MIN_PLOT_HEIGHT);

            let charts = self.args.plots.iter().map(|plot_group| {
                Element::from(
                    ChartWidget::new(SignalChart {
                        store: self.store.clone_ref(),
                        time_range: &self.x_window,
                        end_time: &self.x_end,
                        live_attach: &self.x_live_attach,
                        toplot: plot_group.clone(),
                        theme: self.theme.clone(),
                    })
                    .width(Length::Fill)
                    .height(Length::Fixed(plot_height)),
                )
            });

            scrollable(Column::with_children(charts).spacing(PLOT_SPACING)).into()
        });

        let tables = {
            let nodes = self.store.nodes.read();
            let mut node_names: Vec<_> = nodes.keys().collect();
            node_names.sort();

            let cells = node_names.into_iter().map(|name| -> Element<'_, Message> {
                container(
                    NodeTable::new(name.to_string(), self.store.clone_ref(), self.theme.clone())
                        .view(),
                )
                .width(Length::Shrink)
                .clip(true)
                .into()
            });

            scrollable(
                Row::with_children(cells)
                    .spacing(15)
                    .align_y(alignment::Vertical::Top)
                    .wrap()
                    .vertical_spacing(15),
            )
            .width(Length::Fill)
            .height(Length::Fill)
        };

        let content = Column::new()
            .spacing(5)
            .width(Length::Fill)
            .height(Length::Fill)
            .push(controls)
            .push(status_row);

        let content = match self.current_tab {
            Tab::Plot => content.push(plots),
            Tab::Signals => content.push(tables),
            Tab::Send => content.push(
                MessageSendTable::new(
                    self.store
                        .nodes
                        .read()
                        .values()
                        .flatten()
                        .cloned()
                        .find(|m| m.name() == self.current_send_msg.as_str())
                        .unwrap(),
                    self.store.clone_ref(),
                    self.theme.clone(),
                    self.send_inputs
                        .iter()
                        .filter(|((m, _), _)| *m == self.current_send_msg)
                        .map(|((_, s), v)| (s.clone(), v.clone()))
                        .collect(),
                )
                .view(),
            ),
        };

        content.into()
    }
}

// impl Default for PlotWindow {
//     fn default() -> Self {
//         Self {
//             receiver: None,
//             signals: HashMap::new(),
//             last_redraw: Instant::now(),
//         }
//     }
// }
