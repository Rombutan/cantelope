use iced::{
    Background, Element, Length, Subscription, keyboard, time, widget::Button, widget::Column,
    widget::Row, widget::Text, widget::text_input,
};
use plotters::prelude::*;
use plotters::style::Color;
use plotters_iced2::{Chart, ChartBuilder, ChartWidget};
use std::collections::hash_map::DefaultHasher;
use std::f64;
use std::hash::{Hash, Hasher};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, mpsc::Receiver},
    time::Instant,
};

use iced::widget::{container, text};

pub type DataPoint = (String, f64, f64); // (signal, x, y)

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
    plots: Vec<Vec<String>>,
    regrens: Vec<(String, bool, f64)>,
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
    ThemeChanged(iced::theme::Mode),
    Nothing,
}

pub struct Flags {
    pub receiver: Receiver<DataPoint>,
}

impl Default for Flags {
    fn default() -> Self {
        panic!("Flags::default() should never be used")
    }
}

impl PlotWindow {
    // pub fn run(receiver: Receiver<DataPoint>) -> iced::Result {
    //     <PlotWindow as iced::Application>::run(Settings {
    //         flags: Flags { receiver },
    //         antialiasing: true,
    //         window: iced::window::Settings::default(),
    //         id: None,
    //         default_font: MY_FONT,
    //         default_text_size: 16.0,
    //         exit_on_close_request: true,
    //     })
    // }

    pub fn run(
        receiver: Receiver<DataPoint>,
        _plots: Vec<Vec<String>>,
        _regrens: Vec<(String, bool, f64)>,
    ) -> iced::Result {
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
                            plots: _plots.clone(), // ... Why? WHy? WHY? WHY DOES EVERYTHING NEED TO BE CLONE??? FUCK YOU RUST
                            regrens: _regrens.clone(),
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
        .title("Plots")
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
            Message::Nothing => {}
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
        let controls = Row::new()
            .spacing(10)
            .padding(10)
            .push(Button::new("Pause").on_press(Message::Pause))
            .push(Text::new(format!("Time range in ms:")))
            .push(text_input("3000", &self.time_range_text).on_input(Message::TimeRange));

        let text_color = match self.theme_mode {
            iced::theme::Mode::Dark => RGBColor(255, 255, 255),
            _ => RGBColor(0, 0, 0),
        };

        let status_row = self
            .regrens
            .iter()
            .fold(row![].spacing(10).padding(10), |row, tuple| {
                row.push(SignalStatus::new(tuple, &self.signals).view())
            });

        let charts: Vec<Element<Message>> = self
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

        chart.configure_mesh().draw().unwrap();

        for (_idx, (name, series)) in self.signals.iter().enumerate() {
            if self.toplot.contains(name) {
                let style = Palette99::pick(int_from_str(name)).stroke_width(3);

                chart
                    .draw_series(LineSeries::new(series.iter().copied(), style))
                    .unwrap()
                    .label(name)
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], style));
            }
        }

        let text_color = self.text_color;

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
    name: &'a str,
    // true for >, false for <
    is_greater: bool,
    threshold: f64,
    // The shared signal data
    signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
}

impl<'a> SignalStatus<'a> {
    pub fn new(
        tuple: &'a (String, bool, f64),
        signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
    ) -> Self {
        Self {
            name: &tuple.0,
            is_greater: tuple.1,
            threshold: tuple.2,
            signals,
        }
    }

    pub fn view(&self) -> Element<'a, Message> {
        let latest_val = self
            .signals
            .get(self.name)
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
            iced::Color::from_rgba(0.0, 0.5, 0.0, 0.75) // Green
        } else {
            iced::Color::from_rgba(0.5, 0.0, 0.0, 0.75) // Red
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
