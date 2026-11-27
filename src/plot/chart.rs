use super::Message;
use super::theme::iced_color_to_plotters;
use crate::store;
use plotters::prelude::*;
use plotters_iced2::{Chart, ChartBuilder};
use std::cell::Cell;
use std::fmt::Write;

fn color_from_angle(angle: usize) -> RGBColor {
    // Hue from the angle, 0..360
    let hue = (angle % 360) as f64;
    // Optional: small deterministic variation so neighbors differ more
    let sat = 0.65 + ((angle >> 16) % 12) as f64 / 100.0; // 0.65..0.84
    let lum = 0.45 + ((angle >> 24) % 7) as f64 / 100.0; // 0.45..0.54

    hsl_to_rgb(hue, sat, lum)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> RGBColor {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    RGBColor(
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[derive(Clone, Copy)]
struct PlotLayout {
    left: f32,
    right: f32,
    min_x: f64,
    max_x: f64,
}

impl PlotLayout {
    fn time_at(&self, px: f32) -> f64 {
        let f = ((px - self.left) / (self.right - self.left)).clamp(0.0, 1.0) as f64;
        self.min_x + f * (self.max_x - self.min_x)
    }
    fn frac_at(&self, px: f32) -> f64 {
        ((px - self.left) / (self.right - self.left)).clamp(0.0, 1.0) as f64
    }
}

#[derive(Default)]
pub(super) struct ChartState {
    // Written in build_chart (&State), read in update, hence Cell
    layout: Cell<Option<PlotLayout>>,
    // Drag selection, stored as times so it stays correct if data scrolls
    drag_start: Option<f64>,
    drag_cur: Option<f64>,
}

pub(super) struct SignalChart<'a> {
    pub(super) store: store::TableStore,
    pub(super) time_range: &'a f64,
    pub(super) end_time: &'a f64,
    pub(super) live_attach: &'a bool,
    pub(super) toplot: Vec<String>,
    pub(super) theme: iced::Theme,
}

impl<'a> Chart<Message> for SignalChart<'a> {
    type State = ChartState;

    fn build_chart<DB: DrawingBackend>(&self, state: &Self::State, mut builder: ChartBuilder<DB>) {
        _ = state;

        let mut max_y: f64 = -100000.0;
        let mut min_y: f64 = 100000.0;

        // Take the read lock ONCE. Everything below (index lookups and plotting)
        // uses this same guard, so we never re-lock while already holding it.
        let columns = self.store.read_columns();

        let (start_index, stop_index, min_x, max_x) = match columns.first() {
            Some(store::GenericColumn::F64(c)) if !c.values.is_empty() => {
                let last = c.len() - 1;

                let (start, stop) = if *self.live_attach {
                    // Live: window ends at the newest sample.
                    let now = c.values[last];
                    let start =
                        store::TableStore::index_by_value_locked(&columns, now - *self.time_range);
                    (start, last)
                } else {
                    // Historical: window ends at end_time.
                    let start = store::TableStore::index_by_value_locked(
                        &columns,
                        *self.end_time - *self.time_range,
                    );
                    let stop = store::TableStore::index_by_value_locked(&columns, *self.end_time);
                    (start, stop)
                };

                // index_by_value_locked can return len (past the end), so clamp.
                let start = start.min(last);
                let stop = stop.min(last);

                (start, stop, c.values[start], c.values[stop])
            }
            // Time column exists but has no rows yet, or schema not populated.
            Some(store::GenericColumn::F64(_)) | None => (0, 0, 0.0, 0.0),
            Some(_) => panic!("Time column must be F64"),
        };

        let n = self.toplot.len().max(1);

        let mut chart_lines: Vec<(&String, RGBColor, LineSeries<DB, (f64, f64)>)> = vec![];
        for (i, name) in self.toplot.iter().enumerate() {
            let signal_map = self.store.signalmap.read();

            let Some(&col_index) = signal_map.get(name) else {
                continue;
            };

            if let Some(series) =
                self.store
                    .get_plot_series(&columns, col_index, start_index, stop_index)
            {
                // Evenly spaced: 0, 360/n, 2*360/n, ...
                let color = color_from_angle(i * 360 / n);
                let style = color.stroke_width(3);
                chart_lines.push((
                    name,
                    color,
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

        let xr = chart.plotting_area().get_pixel_range().0;
        if max_x > min_x && xr.end > xr.start {
            state.layout.set(Some(PlotLayout {
                left: xr.start as f32,
                right: xr.end as f32,
                min_x,
                max_x,
            }));
        }

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

        for (name, color, chart_line) in chart_lines {
            let style = color.stroke_width(3);

            // Use series.y_min and series.y_max to construct your Cartesian 2D scale!

            let mut series_name = name.clone().to_string();
            write!(series_name, "              ",).unwrap();
            chart
                .draw_series(chart_line)
                .unwrap()
                .label(series_name)
                .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], style));
        }

        if let (Some(t0), Some(t1)) = (state.drag_start, state.drag_cur) {
            let (a, b) = (t0.min(t1).max(min_x), t0.max(t1).min(max_x));
            if b > a {
                chart
                    .draw_series(std::iter::once(Rectangle::new(
                        [(a, min_y - 0.01), (b, max_y + 0.01)],
                        text_color.mix(0.2).filled(),
                    )))
                    .unwrap();
            }
        }

        chart
            .configure_series_labels()
            .position(SeriesLabelPosition::UpperLeft) // This moves it to the top left
            .background_style(iced_color_to_plotters(self.theme.palette().background))
            .border_style(&TRANSPARENT)
            .label_font(("sans-serif", 12).into_font().color(&text_color))
            .draw()
            .unwrap();
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> (iced::event::Status, Option<Message>) {
        use iced::event::Status;
        use iced::mouse::{Button, Event as Me, ScrollDelta};

        let Some(layout) = state.layout.get() else {
            return (Status::Ignored, None);
        };
        let Some(pos) = cursor.position() else {
            return (Status::Ignored, None);
        };
        let px = pos.x - bounds.x;
        let inside = bounds.contains(pos) && px >= layout.left && px <= layout.right;

        let iced::Event::Mouse(ev) = event else {
            return (Status::Ignored, None);
        };

        match ev {
            Me::ButtonPressed(Button::Left) if inside => {
                let t = layout.time_at(px);
                state.drag_start = Some(t);
                state.drag_cur = Some(t);
                (Status::Captured, None)
            }
            Me::CursorMoved { .. } if state.drag_start.is_some() => {
                state.drag_cur = Some(layout.time_at(px));
                (Status::Captured, None)
            }
            Me::ButtonReleased(Button::Left) if state.drag_start.is_some() => {
                let t0 = state.drag_start.take().unwrap();
                state.drag_cur = None;
                let t1 = layout.time_at(px);

                // Ignore tiny drags
                let per_px = (layout.max_x - layout.min_x) / (layout.right - layout.left) as f64;
                if (t1 - t0).abs() >= 4.0 * per_px {
                    (
                        Status::Captured,
                        Some(Message::SetView {
                            start: t0.min(t1),
                            end: t0.max(t1),
                        }),
                    )
                } else {
                    (Status::Captured, None)
                }
            }
            Me::WheelScrolled { delta } if inside => {
                let y = match delta {
                    ScrollDelta::Lines { y, .. } => *y as f64,
                    ScrollDelta::Pixels { y, .. } => *y as f64 / 50.0,
                };
                if y == 0.0 {
                    return (Status::Ignored, None);
                }
                // Scroll up = zoom in
                let old_window = layout.max_x - layout.min_x;
                let new_window = (old_window * 0.85_f64.powf(y)).max(10.0);

                let frac = layout.frac_at(px);
                let t_cursor = layout.min_x + frac * old_window;
                let start = t_cursor - frac * new_window;

                (
                    Status::Captured,
                    Some(Message::SetView {
                        start,
                        end: start + new_window,
                    }),
                )
            }
            _ => (Status::Ignored, None),
        }
    }
}
