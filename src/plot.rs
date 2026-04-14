use iced::Alignment::Center;
use iced::{
    Background, Element, Length, Size, Subscription, keyboard, time, widget::Button,
    widget::Column, widget::Row, widget::Text, widget::button, widget::text_input,
};

use iced_aw::menu::Menu;
use iced_aw::{menu_bar, menu_items};

use iced::widget::{container, text};
use plotters::prelude::*;
use plotters::style::Color;
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

pub type DataPoint = (String, f64, f64); // (signal, x, y)

use crate::args;
use args::Args;
use std::sync::RwLock;

fn int_from_str(name: &str) -> usize {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();

    // Use modulo to pick an index safely
    hash as usize
}
pub struct PlotWindow {
    receiver: Arc<Mutex<Receiver<DataPoint>>>,
    signals: HashMap<String, VecDeque<(f64, f64)>>,
    last_redraw: Instant,
    args: Arc<RwLock<Args>>,
    play: bool,
    time_range_text: String,
    x_window: f64,
    theme_mode: iced::theme::Mode,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Pause,
    TimeRange(String),
    PlotChanged((usize, String)),
    RegrensChanged(String),
    ThemeChanged(iced::theme::Mode),
    Nothing,
}

impl PlotWindow {
    pub fn run(receiver: Receiver<DataPoint>, _args: Arc<RwLock<Args>>) -> iced::Result {
        let receiver = Arc::new(Mutex::new(receiver));
        iced::application(
            {
                let receiver = Arc::clone(&receiver);
                move || {
                    (
                        PlotWindow {
                            receiver: Arc::clone(&receiver),
                            signals: HashMap::new(),
                            last_redraw: Instant::now(),
                            args: Arc::clone(&_args),
                            play: true,
                            time_range_text: String::from("3000"),
                            x_window: 3000.0,
                            theme_mode: iced::theme::Mode::Light,
                        },
                        iced::system::theme().map(Message::ThemeChanged),
                    )
                }
            },
            PlotWindow::update,
            PlotWindow::view,
        )
        .subscription(PlotWindow::subscription)
        .title("Cantelope Plots")
        .window_size(Size::new(1400.0, 1000.0))
        .centered()
        .run()
    }

    fn ingest_points(&mut self) {
        if let Ok(receiver) = self.receiver.lock() {
            while let Ok((name, x, y)) = receiver.try_recv() {
                let series = self.signals.entry(name).or_default();

                // 1. Add the new point
                series.push_back((x, y));

                // 2. The Delta-X Limit
                // Get the newest X value we just pushed
                let latest_x = x;

                // 3. Remove points from the front while they are outside the 3000-unit window
                while let Some(&(oldest_x, _)) = series.front() {
                    if latest_x - oldest_x > self.x_window {
                        series.pop_front();
                    } else {
                        // The oldest point is now within the window, so stop popping
                        break;
                    }
                }
            }
        }
    }

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
            }
            Message::RegrensChanged(raw_value) => {
                let mut args_lock = self.args.write().unwrap();
                args_lock.regrens_raw = raw_value.clone();

                if raw_value.len() == 0 {
                    args_lock.regrens.clear();
                    args_lock.setup_aux_outputs();
                }

                if let Ok(val) = args::parse_regrens(raw_value) {
                    args_lock.regrens = val;
                    args_lock.setup_aux_outputs();
                }
            }
            Message::Nothing => {}
            _ => {}
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // 1. Your existing timer
        let timer = time::every(std::time::Duration::from_millis(10)).map(|_| Message::Tick);

        // 2. The keyboard listener
        let keyboard = keyboard::listen().map(|event| match event {
            keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Space),
                ..
            } => Message::Pause,
            _ => Message::Nothing,
        });

        Subscription::batch(vec![timer, keyboard])
    }

    fn view(&self) -> Element<'_, Message> {
        use iced::widget::row;
        let args_gaurd = self.args.read().unwrap();

        let menu_tpl = |items| {
            Menu::new(items)
                .width(Length::Fill)
                .offset(0.0)
                .spacing(10.0)
                .close_on_item_click(false)
                .padding(10.0)
        };

        let regren_menu = menu_tpl(menu_items!(
            (
                // Single widget inside the menu
                text_input("Re(d)Gre(e)ns", &self.args.read().unwrap().regrens_raw)
                    .on_input(Message::RegrensChanged)
            )
        ));

        let regren_dropdown = menu_bar!((button("Re(d)Gre(e)ns"), regren_menu));

        let controls = Row::new()
            .spacing(10)
            .padding(10)
            .push(Button::new("Pause").on_press(Message::Pause))
            .push(regren_dropdown)
            .push(Text::new(format!("Period(ms):")).align_y(Center))
            .push(
                text_input("3000", &self.time_range_text)
                    .on_input(Message::TimeRange)
                    .width(100),
            );

        let text_color = match self.theme_mode {
            iced::theme::Mode::Dark => RGBColor(255, 255, 255),
            _ => RGBColor(0, 0, 0),
        };

        let status_row =
            args_gaurd
                .regrens
                .iter()
                .fold(row![].spacing(10).padding(10), |row, tuple| {
                    // .clone() the tuple (or the strings inside it)
                    // so SignalStatus owns the data, not a reference to the lock
                    row.push(SignalStatus::new(tuple.clone(), &self.signals).view())
                });

        let charts: Vec<Element<Message>> = self
            .args
            .read()
            .unwrap()
            .plots
            .iter()
            .map(|plot_group| {
                ChartWidget::new(SignalChart {
                    signals: &self.signals,
                    time_range: &self.x_window,
                    // Pass the inner Vec<String> to the toplot field
                    toplot: plot_group.clone(),
                    text_color,
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into() // Convert each ChartWidget into a generic Element
            })
            .collect();

        let content = Column::new()
            .spacing(5) // Optional: adds a gap between your charts
            .width(Length::Fill)
            .height(Length::Fill)
            .push(controls)
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
    signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
    time_range: &'a f64,
    toplot: Vec<String>,
    pub text_color: RGBColor,
}

impl<'a> Chart<Message> for SignalChart<'a> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, mut builder: ChartBuilder<DB>) {
        _ = state;

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = -100000.0;
        let mut min_y = 100000.0;

        for name in &self.toplot {
            if let Some(series) = self.signals.get(name) {
                for &(_x, y) in series {
                    if max_y < y {
                        max_y = y;
                    }
                    if min_y > y {
                        min_y = y;
                    }
                }
            }
        }

        for series in self.signals.values() {
            for &(x, _y) in series {
                if max_x < x {
                    max_x = x;
                }
                min_x = max_x - self.time_range;
            }
        }

        if !min_x.is_finite() {
            min_x = 0.0;
            max_x = 10.0;
        }

        let chart = builder
            .margin(2)
            .x_label_area_size(40)
            .y_label_area_size(60);

        let mut chart = chart
            .build_cartesian_2d(min_x..max_x, min_y - 0.01..max_y + 0.01)
            .unwrap();

        let text_color = self.text_color;

        chart
            .configure_mesh()
            .label_style(("sans-serif", 12).into_font().color(&text_color))
            .axis_style(&text_color)
            .draw()
            .unwrap();

        for (_idx, (name, series)) in self.signals.iter().enumerate() {
            if self.toplot.contains(name) {
                let style = Palette99::pick(int_from_str(name)).stroke_width(3);

                let mut all_points_iter = series.iter().copied().multipeek();
                let mut cur_seg: Vec<(f64, f64)> = Vec::new();
                let mut last_time = all_points_iter.peek().unwrap_or(&(0.0, 0.0)).0;

                // Find 10th percentile ish gap
                let mut smallest_gaps = [f64::MAX; 10];

                let mut last_t = match all_points_iter.peek() {
                    Some(p) => p.0,
                    None => 0.0,
                };

                for _ in 0..100 {
                    if let Some(p) = all_points_iter.peek() {
                        let gap = p.0 - last_t;
                        last_t = p.0;

                        if gap > 0.0 {
                            // If this gap is smaller than the largest "small" gap we've tracked
                            if gap < smallest_gaps[9] {
                                smallest_gaps[9] = gap;
                                // Keep the 10-element array sorted so index 9 is always the max
                                smallest_gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
                            }
                        }
                    } else {
                        break;
                    }
                }
                all_points_iter.reset_peek();
                let base_gap = if smallest_gaps[9] == f64::MAX {
                    1.0
                } else {
                    smallest_gaps[9]
                };
                let mut tengap = base_gap * 100.0;

                for (t, v) in all_points_iter {
                    if t - last_time > (tengap / 100.0) * 1.5 {
                        // Should break on a missed or highly delayed packet
                        chart
                            .draw_series(LineSeries::new(cur_seg.clone(), style))
                            .unwrap();
                        cur_seg.clear()
                    } else {
                        tengap = tengap * 0.99;
                        tengap = tengap + (t - last_time);
                    }

                    cur_seg.push((t, v));
                    last_time = t;
                }

                let mut series_name = name.clone().to_string();
                write!(series_name, ": {:.1}hz", 1000.0 / (tengap / 100.0)).unwrap();
                chart
                    .draw_series(LineSeries::new(cur_seg.clone(), style))
                    .unwrap()
                    .label(series_name)
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], style));
            }
        }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperLeft) // This moves it to the top left
            .background_style(&TRANSPARENT)
            .border_style(&TRANSPARENT)
            .label_font(("sans-serif", 13).into_font().color(&text_color))
            .draw()
            .unwrap();
    }
}

struct SignalStatus<'a> {
    // The specific signal name to look up
    name: String,
    // true for >, false for <
    is_greater: bool,
    threshold: f64,
    // The shared signal data
    signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
}

impl<'a> SignalStatus<'a> {
    pub fn new(
        tuple: (String, bool, f64),
        signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
    ) -> Self {
        Self {
            name: tuple.0,
            is_greater: tuple.1,
            threshold: tuple.2,
            signals,
        }
    }

    pub fn view(&self) -> Element<'a, Message> {
        let latest_val = self
            .signals
            .get(&self.name)
            .and_then(|series| series.back())
            .map(|(_x, y)| *y)
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
        .style(move |_theme| container::Style {
            background: Some(Background::Color(box_color)),
            text_color: Some(iced::Color::WHITE),
            ..Default::default()
        })
        .into()
    }
}
