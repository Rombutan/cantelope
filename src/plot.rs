use iced::Alignment::Center;
use iced::{
    Background, Element, Length, Size, Subscription, Theme, keyboard, time, widget::Button,
    widget::Column, widget::Row, widget::Text, widget::text_input,
};

use iced::widget::{container, text};
use plotters::prelude::*;
use plotters_iced2::{Chart, ChartBuilder, ChartWidget};
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, mpsc::Receiver},
    time::Instant,
};
use std::{f64, usize};

use itertools::Itertools;

use crate::store;

use crate::args;
use args::Args;

fn int_from_str(name: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();

    // Use modulo to pick an index safely
    hash as usize
}
pub struct PlotWindow {
    store: store::TableStore,
    last_redraw: Instant,
    args: Args,
    play: bool,
    time_range_text: String,
    x_window: f64,
    theme_mode: iced::theme::Mode,
    theme: iced::Theme,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Pause,
    TimeRange(String),
    BumpTime(bool),
    PlotChanged((usize, String)),
    ThemeChanged(iced::theme::Mode),
    Nothing,
}

fn state_to_theme(state: &PlotWindow) -> Theme {
    return mode_to_theme(&state.theme_mode);
}

fn mode_to_theme(theme_mode: &iced::theme::Mode) -> Theme {
    match theme_mode {
        iced::theme::Mode::Dark => {
            return Theme::TokyoNight;
        }
        iced::theme::Mode::Light | iced::theme::Mode::None => {
            return Theme::TokyoNightLight;
        }
    }
}

fn iced_color_to_plotters(iced: iced::Color) -> RGBColor {
    let plotters = {
        RGBColor(
            (iced.r * 255.0) as u8,
            (iced.g * 255.0) as u8,
            (iced.b * 255.0) as u8,
        )
    };
    return plotters;
}

impl PlotWindow {
    pub fn run(store: store::TableStore, _args: Args) -> iced::Result {
        store.wait_until_ready();
        let args = _args.clone();
        iced::application(
            {
                move || {
                    (
                        PlotWindow {
                            store: store.clone_ref(),
                            last_redraw: Instant::now(),
                            args: args.clone(),
                            play: true,
                            time_range_text: String::from("3000"),
                            x_window: 3000.0,
                            theme_mode: iced::theme::Mode::Light,
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
        .title("Cantelope")
        .theme(state_to_theme)
        .window_size(Size::new(1400.0, 1000.0))
        .centered()
        .run()
    }

    fn ingest_points(&mut self) {}

    fn update(&mut self, message: Message) {
        match message {
            Message::Tick => {
                if self.play {
                    self.ingest_points();
                    self.last_redraw = Instant::now();
                }
            }
            Message::Pause => {
                self.play = !self.play;
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
                }
                self.time_range_text = self.x_window.to_string();
            }
            Message::Nothing => {}
            _ => {}
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // 1. Your existing timer
        let timer = time::every(std::time::Duration::from_millis(1000 / 30)).map(|_| Message::Tick);

        // 2. The keyboard listener
        let keyboard = keyboard::listen().map(|event| match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Space),
                ..
            } => Message::Pause,
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Pause),
                ..
            } => Message::Pause,
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowUp),
                ..
            } => Message::BumpTime(true),
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
                ..
            } => Message::BumpTime(false),
            _ => Message::Nothing,
        });

        Subscription::batch(vec![timer, keyboard])
    }

    fn view(&self) -> Element<'_, Message> {
        use iced::widget::row;

        #[cfg(not(feature = "no_control_row"))]
        let controls = Row::new()
            .spacing(10)
            .padding(10)
            .push(Button::new("Pause").on_press(Message::Pause))
            .push(Text::new(format!("Period(ms):")).align_y(Center))
            .push(
                text_input("3000", &self.time_range_text)
                    .on_input(Message::TimeRange)
                    .width(100),
            );

        let status_row =
            self.args
                .regrens
                .iter()
                .fold(row![].spacing(10).padding(10), |row, tuple| {
                    // .clone() the tuple (or the strings inside it)
                    // so SignalStatus owns the data, not a reference to the lock
                    row.push(
                        SignalStatus::new(
                            tuple.clone(),
                            self.store.clone_ref(),
                            self.theme.clone(),
                        )
                        .view(),
                    )
                });

        let charts: Vec<Element<Message>> = self
            .args
            .plots
            .iter()
            .map(|plot_group| {
                ChartWidget::new(SignalChart {
                    store: self.store.clone_ref(),
                    time_range: &self.x_window,
                    // Pass the inner Vec<String> to the toplot field
                    toplot: plot_group.clone(),
                    theme: self.theme.clone(),
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into() // Convert each ChartWidget into a generic Element
            })
            .collect();

        #[cfg(not(feature = "no_control_row"))]
        let content = Column::new()
            .spacing(5) // Optional: adds a gap between your charts
            .width(Length::Fill)
            .height(Length::Fill)
            .push(controls)
            .push(status_row)
            .push(Column::with_children(charts).spacing(5));

        #[cfg(feature = "no_control_row")]
        let content = Column::new()
            .spacing(1)
            .width(Length::Fill)
            .height(Length::Fill)
            .push(status_row)
            .push(Column::with_children(charts).spacing(5));

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

struct SignalChart<'a> {
    store: store::TableStore,
    time_range: &'a f64,
    toplot: Vec<String>,
    pub theme: iced::Theme,
}

impl<'a> Chart<Message> for SignalChart<'a> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, mut builder: ChartBuilder<DB>) {
        _ = state;

        let min_x;
        let max_x;
        let mut max_y: f64 = -100000.0;
        let mut min_y: f64 = 100000.0;

        let start_index;
        let stop_index;
        match &self.store.read_columns()[0] {
            store::GenericColumn::F64(c) => {
                start_index = self
                    .store
                    .find_index_back_by_period(self.time_range.clone());

                stop_index = c.len() - 1;

                if stop_index > 0 {
                    min_x = c.values[start_index];
                    max_x = c.values[stop_index];
                } else {
                    min_x = 0.0;
                    max_x = 0.0;
                }
            }
            _ => panic!("Time column must be F64"),
        };

        let columns = self.store.read_columns();
        let mut chart_lines: Vec<(&String, LineSeries<DB, (f64, f64)>)> = vec![];
        for name in self.toplot.iter() {
            let Ok(signal_map) = self.store.signalmap.read() else {
                continue;
            };

            let Some(&col_index) = signal_map.get(name) else {
                continue;
            };

            if let Some(series) = self.store.get_plot_series(
                &columns,
                col_index, // No need for cloning if we copy/dereference the index
                start_index,
                stop_index,
            ) {
                let style = Palette99::pick(int_from_str(name)).stroke_width(3);
                chart_lines.push((
                    name,
                    plotters::series::LineSeries::new(series.coords, style),
                ));

                if series.y_min < min_y {
                    min_y = series.y_min;
                }

                if series.y_max > max_y {
                    max_y = series.y_max;
                }
            }
        }

        let chart = builder
            .margin(5)
            .x_label_area_size(30)
            .y_label_area_size(60);

        let mut chart = chart
            .build_cartesian_2d(min_x..max_x, min_y - 0.01..max_y + 0.01)
            .unwrap();

        let text_color = iced_color_to_plotters(self.theme.palette().text);

        chart
            .configure_mesh()
            .bold_line_style(&text_color.mix(0.4))
            .light_line_style(&text_color.mix(0.2))
            .set_all_tick_mark_size(-3)
            .max_light_lines(1)
            .y_label_formatter(&|y: &f64| {
                let scaled_val = y;
                let scaled_range = (max_y - min_y) / 1000.0;

                if scaled_val.abs() >= 10000.0
                    || (scaled_range.abs() > 0.0 && scaled_range.abs() < 0.01)
                {
                    format!("{:.2e}", scaled_val)
                } else if scaled_range < 20.0 {
                    format!("{:.1}", scaled_val)
                } else {
                    format!("{:.0}", scaled_val)
                }
            })
            .x_label_formatter(&|x: &f64| {
                let scaled_val = x / 1000.0;

                if max_x - min_x < 100.0 {
                    // Range is small: include 2 decimal places (adjust precision as desired)
                    format!("{:.2}", scaled_val)
                } else {
                    // Range is large: display as a whole integer
                    format!("{:.0}", scaled_val)
                }
            })
            .label_style(("sans-serif", 12).into_font().color(&text_color))
            .axis_style(&text_color)
            .draw()
            .unwrap();

        for (name, chart_line) in chart_lines {
            let style = Palette99::pick(int_from_str(name)).stroke_width(3);

            // Use series.y_min and series.y_max to construct your Cartesian 2D scale!

            let mut series_name = name.clone().to_string();
            write!(
                series_name,
                // Plotters computes the size of the background
                // rectangle to draw behind the legend incorrectly, so...
                ": {:.1}hz              ",
                // 1000.0 / (tengap / 100.0)
                100.0
            )
            .unwrap();
            chart
                .draw_series(chart_line)
                .unwrap()
                .label(series_name)
                .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], style));
        }
        // for (t, v) in all_points_iter {
        //     if t - last_time > (tengap / 100.0) * 1.5 {
        //         // Should break on a missed or highly delayed packet
        //         chart
        //             .draw_series(LineSeries::new(cur_seg.clone(), style))
        //             .unwrap();
        //         cur_seg.clear()
        //     } else {
        //         tengap = tengap * 0.99;
        //         tengap = tengap + (t - last_time);
        //     }

        //     cur_seg.push((t, v));
        //     last_time = t;
        // }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperLeft) // This moves it to the top left
            .background_style(iced_color_to_plotters(self.theme.palette().background))
            .border_style(&TRANSPARENT)
            .label_font(("sans-serif", 12).into_font().color(&text_color))
            .draw()
            .unwrap();
    }
}

struct SignalStatus {
    // The specific signal name to look up
    name: String,
    // true for >, false for <
    is_greater: bool,
    threshold: f64,
    // The shared signal data
    store: store::TableStore,
    theme: iced::Theme,
}

impl<'a> SignalStatus {
    pub fn new(tuple: (String, bool, f64), store: store::TableStore, theme: iced::Theme) -> Self {
        Self {
            name: tuple.0,
            is_greater: tuple.1,
            threshold: tuple.2,
            store: store,
            theme: theme,
        }
    }

    pub fn view(&self) -> Element<'a, Message> {
        let latest_val = self
            .store
            .get_latest_valid_value_by_index(
                self.store
                    .signalmap
                    .read()
                    .unwrap()
                    .get(&self.name)
                    .unwrap()
                    .clone(),
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
