use std::fmt::Write;

use arcnix_core::metric_history::Point as Sample;
use glory::render::PathCommand;
use kartis::{ChartConfig, Curve, Geometry, LineOptions, Margin, Point, Series, line};
use serde::Serialize;

use crate::monitor_ui::Section;

// Unit-square geometry scales to the measured WebView plot. Strokes, markers
// and localized axis labels stay in CSS pixels, including on window resize.
#[derive(Serialize)]
pub(super) struct Plot {
    maximum: f64,
    ticks: Vec<f64>,
    marks: Vec<PlotMark>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum PlotMark {
    Dot {
        series: String,
        x: f64,
        y: f64,
    },
    Line {
        series: String,
        d: String,
        // MonotoneX places its two control points at 1/3 and 2/3 of
        // the time interval. The live tip uses their actual Kartis heights.
        #[serde(skip_serializing_if = "Option::is_none")]
        tip_y: Option<[f64; 2]>,
    },
    Area {
        series: String,
        d: String,
    },
}

fn series(name: &str, samples: &[Sample], value: impl Fn(&Sample) -> Option<f64>) -> Series {
    let mut data = Vec::with_capacity(samples.len());
    for sample in samples {
        if sample.break_before && !data.is_empty() {
            data.push(Point::gap(sample.at));
        }
        data.push(Point {
            x: sample.at,
            y: value(sample),
        });
    }
    Series::new(name, data)
}

pub(super) fn layout(kind: Section, samples: &[Sample], domain: [f64; 2]) -> kartis::Result<Plot> {
    let series = match kind {
        Section::Cpu => vec![series("cpu", samples, |p| p.cpu)],
        Section::Memory => vec![series("memory", samples, |p| p.memory)],
        Section::Network => vec![
            series("receive", samples, |p| p.receive),
            series("transmit", samples, |p| p.transmit),
        ],
        _ => unreachable!("only resource trend sections have plots"),
    };
    layout_series(
        series,
        samples,
        domain,
        (kind != Section::Network).then_some(100.),
        kind != Section::Network,
    )
}

pub(super) fn gpu_layout(
    samples: &[Sample],
    domain: [f64; 2],
    id: &str,
    key: &str,
    maximum: Option<f64>,
) -> kartis::Result<Plot> {
    let values = series(key, samples, |p| {
        p.gpus.iter().find(|g| g.id == id)?.value(key)
    });
    layout_series(vec![values], samples, domain, maximum, true)
}

fn layout_series(
    series: Vec<Series>,
    samples: &[Sample],
    domain: [f64; 2],
    fixed_max: Option<f64>,
    area: bool,
) -> kartis::Result<Plot> {
    let (maximum, ticks) = if let Some(maximum) = fixed_max.filter(|n| *n > 0.) {
        (maximum, (0..=4).map(|i| maximum * i as f64 / 4.).collect())
    } else {
        let peak = series
            .iter()
            .flat_map(|s| &s.data)
            .filter_map(|p| p.y)
            .fold(4., f64::max);
        let scale = kartis::scales::LinearScale::new([0., peak], [1., 0.])?.nice(4);
        (scale.domain[1], scale.ticks(4))
    };
    let config = ChartConfig {
        width: 1.,
        height: 1.,
        margin: Margin {
            top: 0.,
            right: 0.,
            bottom: 0.,
            left: 0.,
        },
        ..Default::default()
    };
    let mut options = LineOptions {
        x_domain: Some(domain),
        y_domain: Some([0., maximum]),
        points: false,
        area,
        // Reads can share a clock tick. Monotone interpolation requires strictly increasing x.
        curve: if samples.windows(2).all(|w| w[0].at < w[1].at) {
            Curve::MonotoneX
        } else {
            Curve::Linear
        },
        ..Default::default()
    };
    options.axes.x.visible = false;
    options.axes.y.visible = false;
    options.axes.y.grid = false;
    let scene = line(&config, &series, &options)?;
    let mut marks = Vec::with_capacity(scene.marks.len());
    for mark in scene.marks {
        let Geometry::Path(commands) = mark.geometry else {
            continue;
        };
        let series = mark.series;
        if let [PathCommand::Move(p)] | [PathCommand::Move(p), PathCommand::Close] =
            commands.as_slice()
        {
            marks.push(PlotMark::Dot {
                series,
                x: p.x,
                y: p.y,
            });
            continue;
        }
        // Bound IPC size for a full 15-minute window. Five fractional digits
        // in a unit square give subpixel precision even on a 4K display.
        let tip_y = match commands.last() {
            Some(PathCommand::Cubic { first, second, end }) if end.x == 1. => {
                Some([first.y, second.y])
            }
            _ => None,
        };
        let mut d = String::new();
        for command in commands {
            match command {
                PathCommand::Move(p) => write!(d, "M{:.5} {:.5}", p.x, p.y),
                PathCommand::Line(p) => write!(d, "L{:.5} {:.5}", p.x, p.y),
                PathCommand::Cubic { first, second, end } => write!(
                    d,
                    "C{:.5} {:.5} {:.5} {:.5} {:.5} {:.5}",
                    first.x, first.y, second.x, second.y, end.x, end.y
                ),
                PathCommand::Close => d.write_char('Z'),
            }
            .expect("writing to String");
        }
        marks.push(if mark.fill.is_some() {
            PlotMark::Area { series, d }
        } else {
            PlotMark::Line { series, d, tip_y }
        });
    }
    Ok(Plot {
        maximum,
        ticks,
        marks,
    })
}

#[cfg(test)]
mod tests {
    use arcnix_core::metric_history::MAX_POINTS;

    use super::*;

    fn sample(at: f64) -> Sample {
        Sample {
            at,
            cpu: None,
            memory: Some(50.),
            receive: Some(0.),
            transmit: Some(2048.),
            gpus: Vec::new(),
            break_before: false,
        }
    }

    #[test]
    fn repeated_timestamps_and_isolated_reads_survive_layout() {
        let mut samples = [sample(0.), sample(0.), sample(1.)];
        samples[1].memory = Some(75.);
        samples[2].break_before = true;
        let plot = layout(Section::Memory, &samples, [0., 1.]).unwrap();
        assert!(
            plot.marks
                .iter()
                .any(|m| matches!(m, PlotMark::Line { .. }))
        );
        assert!(
            plot.marks
                .iter()
                .any(|m| matches!(m, PlotMark::Dot { x: 1., y: 0.5, .. }))
        );
    }

    #[test]
    fn full_window_keeps_both_network_series_across_path_chunks() {
        let samples: Vec<_> = (0..MAX_POINTS).map(|i| sample(i as f64 / 2.)).collect();
        let plot = layout(Section::Network, &samples, [0., samples.last().unwrap().at]).unwrap();
        assert!(plot.maximum >= 2048.);
        for name in ["receive", "transmit"] {
            assert!(
                plot.marks
                    .iter()
                    .filter(|m| matches!(m, PlotMark::Line { series, .. } if series == name))
                    .count()
                    > 1
            );
        }
    }
}
