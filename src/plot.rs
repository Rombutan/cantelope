use iced::time;
use iced::{Element, Length, Subscription, widget::Column};
use plotters::prelude::*;
use plotters_iced2::{Chart, ChartBuilder, ChartWidget};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, mpsc::Receiver},
    time::{Duration, Instant},
};

pub type DataPoint = (String, f64, f64); // (signal, x, y)

const X_WINDOW: f64 = 3000.0;
const FPS_LIMIT: u64 = 25;

pub struct PlotWindow {
    receiver: Arc<Mutex<Receiver<DataPoint>>>,
    signals: HashMap<String, VecDeque<(f64, f64)>>,
    last_redraw: Instant,
    plots: Vec<Vec<String>>,
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

    pub fn run(receiver: Receiver<DataPoint>, _plots: Vec<Vec<String>>) -> iced::Result {
        let receiver = Arc::new(Mutex::new(receiver));

        iced::application(
            {
                let receiver = Arc::clone(&receiver);
                move || PlotWindow {
                    receiver: Arc::clone(&receiver),
                    signals: HashMap::new(),
                    last_redraw: Instant::now(),
                    plots: _plots.clone(), // ... Why? WHy? WHY? WHY DOES EVERYTHING NEED TO BE CLONE??? FUCK YOU RUST
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
                    if latest_x - oldest_x > 10000.0 {
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
                // FPS cap
                if self.last_redraw.elapsed() >= Duration::from_millis(1000 / FPS_LIMIT) {
                    self.ingest_points();
                    self.last_redraw = Instant::now();
                }
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(std::time::Duration::from_millis(40)).map(|_| Message::Tick) // 25 FPS
    }

    fn view(&self) -> Element<'_, Message> {
        // 1. Map over the outer Vec to create a list of widgets
        let charts: Vec<Element<Message>> = self
            .plots
            .iter()
            .map(|plot_group| {
                ChartWidget::new(SignalChart {
                    signals: &self.signals,
                    // Pass the inner Vec<String> to the toplot field
                    toplot: plot_group.clone(),
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into() // Convert each ChartWidget into a generic Element
            })
            .collect();

        // 2. Place all charts into a Column for a vertical layout
        let content = Column::with_children(charts)
            .spacing(10) // Optional: adds a gap between your charts
            .width(Length::Fill)
            .height(Length::Fill);

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
    toplot: Vec<String>,
}

impl<'a> Chart<Message> for SignalChart<'a> {
    type State = ();

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, mut builder: ChartBuilder<DB>) {
        _ = state;

        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = 0.01;
        let mut min_y = -0.01;

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
                if min_x > x {
                    min_x = x;
                }

                if max_x < x {
                    max_x = x;
                }
            }
        }

        if !min_x.is_finite() {
            min_x = 0.0;
            max_x = X_WINDOW;
        }

        let chart = builder
            .margin(10)
            .x_label_area_size(40)
            .y_label_area_size(60);

        let mut chart = chart
            .build_cartesian_2d(min_x..max_x, -max_y..max_y)
            .unwrap();

        chart.configure_mesh().draw().unwrap();

        for (idx, (name, series)) in self.signals.iter().enumerate() {
            if self.toplot.contains(name) {
                let color = Palette99::pick(idx);

                chart
                    .draw_series(LineSeries::new(series.iter().copied(), &color))
                    .unwrap()
                    .label(name)
                    .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &color));
            }
        }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperLeft) // This moves it to the top left
            .background_style(&TRANSPARENT)
            .border_style(&TRANSPARENT)
            .draw()
            .unwrap();
    }
}
