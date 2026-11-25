//! Charts consume Rust-owned, session-only samples; they never fetch data.
mod plot;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::btn;
use crate::*;

pub(super) const GPU_CHARTS: [(&str, &str); 9] = [
    ("graphics", "3D"),
    ("renderer", "Renderer utilization"),
    ("tiler", "Tiler utilization"),
    ("copy", "Copy"),
    ("encode", "Video encode"),
    ("decode", "Video decode"),
    ("utilization", "GPU utilization"),
    ("dedicated", "Dedicated GPU memory"),
    ("shared", "Shared GPU memory"),
];

fn payload(s: &State, section: monitor_ui::Section) -> String {
    let m = &s.metrics;
    let range = match m.window {
        60 | 900 => m.window,
        _ => 300,
    };
    let label = |key| crate::i18n::translate_for(key, desktop_system_settings::language(s));
    let points = m.history.window(range);
    let end = points.last().map_or(0., |p| p.at);
    let span = end.clamp(1., f64::from(range));
    let domain = [end - span, end];
    let mut plots = serde_json::Map::new();
    for (kind, name) in [
        (monitor_ui::Section::Cpu, "cpu"),
        (monitor_ui::Section::Memory, "memory"),
        (monitor_ui::Section::Network, "network"),
    ] {
        if section == monitor_ui::Section::Overview || section == kind {
            plots.insert(
                name.into(),
                serde_json::json!(plot::layout(kind, &points, domain).ok()),
            );
        }
    }
    let gpu = monitor_ui::selected_gpu(s).filter(|_| section == monitor_ui::Section::Gpu);
    let mut samples = serde_json::to_value(&points).unwrap_or_default();
    let mut gpu_labels = serde_json::Map::new();
    if let Some(g) = gpu {
        for (key, title) in GPU_CHARTS {
            if g.value(key).is_none() || (key == "utilization" && g.graphics.is_some()) {
                continue;
            }
            let maximum = match key {
                "dedicated" => g.dedicated_total.map(|v| v as f64),
                "shared" => g.shared_total.map(|v| v as f64),
                _ => Some(100.),
            };
            plots.insert(
                key.into(),
                serde_json::json!(plot::gpu_layout(&points, domain, &g.id, key, maximum).ok()),
            );
            gpu_labels.insert(key.into(), label(title).into());
            for (row, p) in samples.as_array_mut().into_iter().flatten().zip(&points) {
                row[key] = serde_json::json!(
                    p.gpus
                        .iter()
                        .find(|card| card.id == g.id)
                        .and_then(|card| card.value(key))
                );
            }
        }
    }
    serde_json::json!({
        "gpuLabels":gpu_labels,"adapter":gpu.map(|g|&g.id),
        "range":range,"points":samples,"domain":domain,"plots":plots,
        "labels":{
            "cpu":label("CPU usage"),"memory":label("Memory usage"),"network":label("Network rate"),
            "receive":label("Receive"),"transmit":label("Transmit"),"missing":label("Unavailable"),
            "empty":label("Waiting for samples"),"latest":label("Latest read"),"seconds":label("s"),"minutes":label("min"),
        },
        "base":if s.config.settings.display.units==arcnix_core::preferences::SizeUnits::Decimal{1000}else{1024},
        "decimals":s.config.settings.display.decimals,
        "grouping":s.config.settings.display.group_digits,
        "locale":if desktop_system_settings::language(s)==arcnix_core::Language::SimplifiedChinese{"zh-CN"}else{"en-US"},
    }).to_string()
}
pub(crate) fn panel(ui: Ui, section: monitor_ui::Section) -> impl Widget {
    let state = ui.state;
    let resume = ui.clone();
    // Desktop state also changes for pointer, task and window events. Only lay
    // out the history again when its samples or presentation actually change.
    let cache = std::cell::RefCell::new(None);
    let chart_data = state.map(move |s| {
        let key = (
            s.metrics.completed,
            s.metrics.gpu_id.clone(),
            s.metrics.origin,
            s.metrics.window,
            s.config.settings.display,
            desktop_system_settings::language(s),
        );
        let mut cache = cache.borrow_mut();
        if let Some((last, value)) = cache.as_ref()
            && *last == key
        {
            return String::clone(value);
        }
        let value = payload(s, section);
        *cache = Some((key, value.clone()));
        value
    });
    col().class("resource-trends").fill((
        row().class("resource-trend-toolbar").fill((
            row().class("help-heading").fill((
                text().class("section-title").content(i18n::bind("Session trends")),
                crate::help_ui::tip_raw("Session trends", i18n::bind(
                    "Hover or use the left and right arrow keys to inspect a sample. Gaps indicate missing data.",
                )),
            )),
            row().class("script-actions resource-trend-windows").fill(each(vec![(60u16,"1 min"),(300,"5 min"),(900,"15 min")]).key(|(n,_)|*n).children(move|item|{let (seconds,label)=item.value.get();btn(label,move||state.revise(|s|s.metrics.window=seconds)).class(state.map(move|s|if (if s.metrics.window==0{300}else{s.metrics.window})==seconds{"choice active"}else{"choice"}.to_owned()))})),
            btn("Clear history",move||state.revise(|s|{s.metrics.history=Default::default();s.metrics.origin=None;observability::metrics_leave(s);})),
            sapid::button().class("button").label(i18n::bind(state.map(|s|if s.metrics.paused{"Resume sampling"}else{"Pause sampling"}.to_owned()))).on_press(move||{
                let paused=state.revise(|s|{s.metrics.paused= !s.metrics.paused;observability::metrics_leave(s);s.metrics.paused});
                if !paused{resume.metrics_refresh();}
            }),
        )),
        text().class("resource-trend-data hidden").content(chart_data),
        view().class(match section {
            monitor_ui::Section::Gpu => "resource-trend-plots plot-gpu",
            monitor_ui::Section::Cpu => "resource-trend-plots plot-cpu",
            monitor_ui::Section::Memory => "resource-trend-plots plot-memory",
            monitor_ui::Section::Network => "resource-trend-plots plot-network",
            _ => "resource-trend-plots",
        }),
    ))
}
