//! The media tools prompt (`DialogModel::MediaTools`, Phase V): the GUI's
//! `ui/dialogs/media_tools.rs` on the core's `MediaToolsModel`. The form starts with the
//! tool (a radio group: Left / Right switch, which rebuilds the fields below), then the
//! tool's fields (Convert: the preset, cycled with Left / Right and listed above the form
//! with the picked one marked, a click on a row picks it; Extract audio: the codec;
//! Trim: start, end, "cut on keyframes"; Save frame: time, picture format), the output
//! name and what to do when it is taken. Enter runs (`MediaToolsModel::submit`, once
//! the model finds no error, which shows under the fields), Esc cancels.
//!
//! [`prepare`] runs every frame while the prompt is on top: it moves the worker results
//! (the probe, the plan) into the model and copies an automatically changed end time or
//! output name into the form.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::models::media_tools::{
    AUDIO_CODECS, ConflictChoice, ImageFormat, MediaTool, MediaToolsModel, codec_label, preset_label,
};
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;

use super::prompt::{buttons, click, is_key, left_click, set_text, wheel};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::layout::HitTarget;
use crate::widgets::{Field, Form, FormOutcome, SelectList};

/// The tool radio group is always the first field.
pub const TOOL: usize = 0;
/// The Run button (the other one cancels).
const GO: usize = 1;

/// Which field index holds what, for the current tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Tool,
    Preset,
    Codec,
    Start,
    End,
    StreamCopy,
    At,
    Format,
    Name,
    Conflict,
}

/// The form's fields for `m.tool`, in order.
pub fn slots(tool: MediaTool) -> Vec<Slot> {
    let mut v = vec![Slot::Tool];
    match tool {
        MediaTool::Convert => v.push(Slot::Preset),
        MediaTool::ExtractAudio => v.push(Slot::Codec),
        MediaTool::Trim => v.extend([Slot::Start, Slot::End, Slot::StreamCopy]),
        MediaTool::SaveFrame => v.extend([Slot::At, Slot::Format]),
    }
    v.extend([Slot::Name, Slot::Conflict]);
    v
}

fn slot_index(tool: MediaTool, slot: Slot) -> Option<usize> {
    slots(tool).iter().position(|s| *s == slot)
}

fn preset_field(m: &MediaToolsModel) -> Field {
    let label = m.presets.get(m.preset).map(|p| preset_label(p.id).to_string()).unwrap_or_default();
    Field::radio(tr!("dialogs-media-preset"), vec![label], 0)
}

/// The form for the model's current tool and values.
pub fn form(m: &MediaToolsModel) -> Form {
    let tool_labels: Vec<String> = MediaTool::ALL.iter().map(|t| t.label().to_string()).collect();
    let tool_sel = MediaTool::ALL.iter().position(|t| *t == m.tool).unwrap_or(0);
    let mut fields = Vec::new();
    for slot in slots(m.tool) {
        fields.push(match slot {
            Slot::Tool => Field::radio(tr!("dialogs-media-tool"), tool_labels.clone(), tool_sel),
            Slot::Preset => preset_field(m),
            Slot::Codec => Field::radio(
                tr!("dialogs-media-codec"),
                AUDIO_CODECS.iter().map(|c| codec_label(*c).to_string()).collect(),
                AUDIO_CODECS.iter().position(|c| *c == m.codec).unwrap_or(0),
            ),
            Slot::Start => Field::text(tr!("dialogs-media-start"), m.start.clone()),
            Slot::End => Field::text(tr!("dialogs-media-end"), m.end.clone()),
            Slot::StreamCopy => Field::check(tr!("dialogs-media-stream-copy"), m.stream_copy),
            Slot::At => Field::text(tr!("dialogs-media-at"), m.at.clone()),
            Slot::Format => Field::radio(
                tr!("dialogs-media-format"),
                vec!["PNG".into(), "JPEG".into()],
                usize::from(m.format == ImageFormat::Jpeg),
            ),
            Slot::Name => Field::text(tr!("dialogs-media-name"), m.name.clone()),
            Slot::Conflict => Field::radio(
                tr!("dialogs-media-exists"),
                ConflictChoice::ALL.iter().map(|c| c.label().to_string()).collect(),
                ConflictChoice::ALL.iter().position(|c| *c == m.conflict).unwrap_or(0),
            ),
        });
    }
    let mut f = Form::new(fields, buttons(&[tr!("common-cancel"), m.tool.go_label()])).default_button(GO);
    f.focus = slot_index(m.tool, Slot::Name).unwrap_or(0);
    f.error = m.error();
    f
}

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::MediaTools(t) = m else { return ModelUi::default() };
    let mut list = SelectList::default();
    list.select(t.preset, t.presets.len());
    ModelUi { form: form(t), list, ..ModelUi::default() }
}

/// Every frame while the prompt is on top: worker results into the model, automatic
/// values into the form.
pub fn prepare(t: &mut MediaToolsModel, ui: &mut ModelUi) {
    if !t.sync() {
        return;
    }
    for (slot, text) in [(Slot::End, &t.end), (Slot::Name, &t.name)] {
        if let Some(i) = slot_index(t.tool, slot)
            && ui.form.fields.get(i).and_then(|f| f.value()) != Some(text.as_str())
        {
            set_text(&mut ui.form, i, text);
        }
    }
    ui.form.error = t.error();
}

/// Rebuild the form after the tool changed (the focus goes to the tool group).
fn rebuild(t: &MediaToolsModel, ui: &mut ModelUi) {
    ui.form = form(t);
    ui.form.focus = TOOL;
    ui.list = SelectList::default();
    ui.list.select(t.preset, t.presets.len());
}

/// Pick preset `i`: the model, the field's label and the list cursor.
fn pick_preset(t: &mut MediaToolsModel, ui: &mut ModelUi, i: usize) {
    t.set_preset(i);
    if let Some(k) = slot_index(t.tool, Slot::Preset) {
        ui.form.fields[k] = preset_field(t);
    }
    ui.list.select(t.preset, t.presets.len());
    if let Some(i) = slot_index(t.tool, Slot::Name) {
        let name = t.name.clone();
        set_text(&mut ui.form, i, &name);
    }
    ui.form.error = t.error();
}

fn text_of(ui: &ModelUi, i: usize) -> String {
    ui.form.fields.get(i).and_then(|f| f.value()).unwrap_or_default().to_string()
}

fn on_form(t: &mut MediaToolsModel, ui: &mut ModelUi, out: FormOutcome) -> Option<Outcome> {
    match out {
        FormOutcome::Changed(i) => {
            let slot = slots(t.tool).get(i).copied();
            match slot {
                Some(Slot::Tool) => {
                    if let Some(k) = ui.form.fields[i].selected().and_then(|k| MediaTool::ALL.get(k).copied()) {
                        t.set_tool(k);
                        rebuild(t, ui);
                        return None;
                    }
                }
                Some(Slot::Codec) => {
                    if let Some(c) = ui.form.fields[i].selected().and_then(|k| AUDIO_CODECS.get(k).copied()) {
                        t.set_codec(c);
                    }
                }
                Some(Slot::Start) => t.set_start(&text_of(ui, i)),
                Some(Slot::End) => t.set_end(&text_of(ui, i)),
                Some(Slot::StreamCopy) => t.set_stream_copy(ui.form.fields[i].on().unwrap_or(true)),
                Some(Slot::At) => t.set_at(&text_of(ui, i)),
                Some(Slot::Format) => t.set_format(if ui.form.fields[i].selected() == Some(1) {
                    ImageFormat::Jpeg
                } else {
                    ImageFormat::Png
                }),
                Some(Slot::Name) => t.set_name(&text_of(ui, i)),
                Some(Slot::Conflict) => {
                    if let Some(c) = ui.form.fields[i].selected().and_then(|k| ConflictChoice::ALL.get(k).copied()) {
                        t.set_conflict(c);
                    }
                }
                Some(Slot::Preset) | None => {}
            }
            // The automatic output name follows the operation.
            if let Some(k) = slot_index(t.tool, Slot::Name)
                && k != i
                && ui.form.fields[k].value() != Some(t.name.as_str())
            {
                let name = t.name.clone();
                set_text(&mut ui.form, k, &name);
            }
            ui.form.error = t.error();
            None
        }
        FormOutcome::Pressed(GO) => match t.submit() {
            Ok(cmd) => Some(Outcome::Close(Some(cmd))),
            Err(e) => {
                ui.form.error = Some(e);
                None
            }
        },
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Consumed | FormOutcome::Ignored => None,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    let DialogModel::MediaTools(t) = m else { return Outcome::Keep };
    for ev in &i.events {
        // Left / Right on the preset field cycle the presets (one option is shown).
        if t.tool == MediaTool::Convert
            && ui.form.focused_field() == slot_index(t.tool, Slot::Preset)
            && !t.presets.is_empty()
        {
            let n = t.presets.len();
            if is_key(ev, KeyName::ArrowRight) || is_key(ev, KeyName::Space) {
                pick_preset(t, ui, (t.preset + 1) % n);
                continue;
            }
            if is_key(ev, KeyName::ArrowLeft) {
                pick_preset(t, ui, (t.preset + n - 1) % n);
                continue;
            }
        }
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(t, ui, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: a click on a preset row picks it.
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let DialogModel::MediaTools(t) = m else { return Outcome::Keep };
    if t.tool == MediaTool::Convert {
        if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
            if r < t.presets.len() {
                pick_preset(t, ui, r);
            }
            return Outcome::Keep;
        }
        if wheel(&mut ui.list, e, t.presets.len()) {
            return Outcome::Keep;
        }
    }
    let out = click(&mut ui.form, e);
    on_form(t, ui, out).unwrap_or(Outcome::Keep)
}

/// The prompt never goes away by itself.
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_per_tool() {
        assert_eq!(slots(MediaTool::Convert), vec![Slot::Tool, Slot::Preset, Slot::Name, Slot::Conflict]);
        assert_eq!(
            slots(MediaTool::Trim),
            vec![Slot::Tool, Slot::Start, Slot::End, Slot::StreamCopy, Slot::Name, Slot::Conflict]
        );
        assert_eq!(slot_index(MediaTool::SaveFrame, Slot::Name), Some(3));
        assert_eq!(slot_index(MediaTool::Convert, Slot::Start), None);
    }

    // The form against a live model needs loaded FFmpeg libraries: see the with-libraries
    // suite in `soyuz-appcore/tests/media_tools.rs`.
}
