use iced::Font;
use iced::time;
use iced::{Application, Command, Element, Length, Settings, Subscription, Theme};
use plotters::prelude::*;
use plotters_iced::{Chart, ChartBuilder, ChartWidget};
use std::{
    collections::{HashMap, VecDeque},
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};
const MY_FONT: Font = Font::with_name("Arial"); // Example
pub type DataPoint = (String, f64, f64); // (signal, x, y)

const X_WINDOW: f64 = 5000.0;
const FPS_LIMIT: u64 = 25;

pub struct PlotWindow {
    receiver: Receiver<DataPoint>,
    signals: HashMap<String, VecDeque<(f64, f64)>>,
    last_redraw: Instant,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
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
    pub fn run(receiver: Receiver<DataPoint>) -> iced::Result {
        <PlotWindow as iced::Application>::run(Settings {
            flags: Flags { receiver },
            antialiasing: true,
            window: iced::window::Settings::default(),
            id: None,
            default_font: MY_FONT,
            default_text_size: 16.0,
            exit_on_close_request: true,
        })
    }

    fn run_with_settings(settings: Settings<()>, receiver: Receiver<DataPoint>) -> iced::Result {
        PlotWindow::run(receiver)
    }

    fn new(receiver: Receiver<DataPoint>) -> (Self, Command<Message>) {
        (
            Self {
                receiver,
                signals: HashMap::new(),
                last_redraw: Instant::now(),
            },
            Command::none(),
        )
    }

    fn ingest_points(&mut self) {
        while let Ok((name, x, y)) = self.receiver.try_recv() {
            let series = self.signals.entry(name).or_insert_with(VecDeque::new);

            series.push_back((x, y));

            // Drop old X values outside the window
            while let Some((old_x, _)) = series.front() {
                if x - *old_x > X_WINDOW {
                    series.pop_front();
                } else {
                    break;
                }
            }
        }
    }
}

impl Application for PlotWindow {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = Flags;

    fn new(flags: Flags) -> (Self, Command<Message>) {
        (
            Self {
                receiver: flags.receiver,
                signals: HashMap::new(),
                last_redraw: Instant::now(),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "Live Signal Plot".into()
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Tick => {
                // FPS cap
                if self.last_redraw.elapsed() >= Duration::from_millis(1000 / FPS_LIMIT) {
                    self.ingest_points();
                    self.last_redraw = Instant::now();
                }
            }
        }
        Command::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_millis(10)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<Message> {
        let chart = ChartWidget::new(SignalChart {
            signals: &self.signals,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        chart.into()
    }
}

struct SignalChart<'a> {
    signals: &'a HashMap<String, VecDeque<(f64, f64)>>,
}

impl<'a> Chart<Message> for SignalChart<'a> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, mut builder: ChartBuilder<DB>) {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = 1.0;

        for series in self.signals.values() {
            for &(x, y) in series {
                if min_x > x {
                    min_x = x;
                }
                if max_x < x {
                    max_x = x;
                }
                if max_y < y {
                    max_y = y;
                }
            }
        }

        if !min_x.is_finite() {
            min_x = 0.0;
            max_x = X_WINDOW;
        }

        let chart = builder
            .margin(10)
            .x_label_area_size(30)
            .y_label_area_size(40);

        let mut chart = chart
            .build_cartesian_2d(min_x..max_x, -max_y..max_y)
            .unwrap();

        chart.configure_mesh().draw().unwrap();

        for (idx, (name, series)) in self.signals.iter().enumerate() {
            let color = Palette99::pick(idx);

            chart
                .draw_series(LineSeries::new(series.iter().copied(), &color))
                .unwrap()
                .label(name)
                .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &color));
        }

        chart
            .configure_series_labels()
            .border_style(&BLACK)
            .draw()
            .unwrap();
    }
}
