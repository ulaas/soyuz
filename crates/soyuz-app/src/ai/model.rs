//! Model abstraction for the harness: [`ChatModel`] over a configured
//! `soyuz_ai::Provider` (or the scripted [`super::fake::FakeModel`]), plus model routing:
//! a fast (cheap / local) model for naming and classifying, a strong one for planning.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use soyuz_ai::{ChatRequest, Event, NetSettings, Provider, StreamHandle, ToolProtocol};
use soyuz_core::config::{Ai, AiProfile, AiToolMode};

use super::Wake;

/// Environment variable naming a fake-provider transcript (demos, manual testing).
pub const FAKE_ENV: &str = "SOYUZ_AI_FAKE";

/// Which model a task should use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// Naming, classifying, one-shot answers (`ai.tasks.fast`).
    Fast,
    /// Planning multi-step file work (`ai.tasks.strong`).
    Strong,
}

/// A streamed model answer.
pub struct ModelStream {
    pub rx: Receiver<Event>,
    cancel: Box<dyn Fn() + Send>,
    _keep: Option<StreamHandle>,
}

impl ModelStream {
    pub fn new(rx: Receiver<Event>, cancel: impl Fn() + Send + 'static) -> Self {
        ModelStream { rx, cancel: Box::new(cancel), _keep: None }
    }

    pub fn cancel(&self) {
        (self.cancel)();
    }
}

/// A chat model the agent can stream from.
pub trait ChatModel: Send + Sync {
    /// Profile name (budget and ledger key).
    fn profile(&self) -> &str;
    fn model(&self) -> &str;
    /// Runs on this machine (no redaction needed, no content leaves the box).
    fn is_local(&self) -> bool;
    fn protocol(&self) -> ToolProtocol;
    fn max_tokens(&self) -> u32;
    /// The model accepts images in user messages (the profile's `vision` switch).
    fn accepts_images(&self) -> bool {
        false
    }
    fn stream(&self, req: ChatRequest, wake: Wake) -> ModelStream;
}

/// A real provider.
pub struct ProviderModel {
    provider: Provider,
    profile: String,
    local: bool,
    vision: bool,
}

impl ProviderModel {
    pub fn new(provider: Provider, profile: &AiProfile) -> Self {
        ProviderModel {
            local: profile.kind.is_local(),
            profile: profile.name.clone(),
            provider,
            vision: profile.vision,
        }
    }
}

impl ChatModel for ProviderModel {
    fn profile(&self) -> &str {
        &self.profile
    }

    fn model(&self) -> &str {
        &self.provider.model
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn protocol(&self) -> ToolProtocol {
        match self.provider.tool_mode {
            AiToolMode::Json => ToolProtocol::Json,
            AiToolMode::Native | AiToolMode::Auto => ToolProtocol::Native,
        }
    }

    fn max_tokens(&self) -> u32 {
        self.provider.max_tokens.max(256)
    }

    fn accepts_images(&self) -> bool {
        self.vision
    }

    fn stream(&self, req: ChatRequest, wake: Wake) -> ModelStream {
        let handle = self.provider.stream(req, wake);
        let token = handle.token();
        ModelStream { rx: handle.rx.clone(), cancel: Box::new(move || token.cancel()), _keep: Some(handle) }
    }
}

/// The profile and model id a route resolves to (without touching the credential store).
pub fn resolve(ai: &Ai, route: Route) -> Option<(AiProfile, String)> {
    let task = match route {
        Route::Fast => &ai.tasks.fast,
        Route::Strong => &ai.tasks.strong,
    };
    let profile = if task.profile.trim().is_empty() { ai.default_profile() } else { ai.profile(task.profile.trim()) }
        .or_else(|| ai.default_profile())?
        .clone();
    let model = if task.model.trim().is_empty() { profile.model.clone() } else { task.model.trim().to_string() };
    Some((profile, model))
}

/// The fake transcript from the environment, if set.
pub fn fake_transcript() -> Option<PathBuf> {
    std::env::var_os(FAKE_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// What the chips show for a route (`profile · model`), without building anything.
pub fn route_label(ai: &Ai, route: Route) -> Option<(String, String)> {
    if let Some(t) = fake_transcript() {
        let name = t.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        return Some(("fake".into(), name));
    }
    resolve(ai, route).map(|(p, m)| (p.name, m))
}

/// Build the model for a route. Reads the credential store: call on a worker thread.
pub fn build(ai: &Ai, route: Route, vars: &HashMap<String, String>) -> Result<Arc<dyn ChatModel>, String> {
    if let Some(path) = fake_transcript() {
        return super::fake::FakeModel::from_file(&path, vars).map(|m| Arc::new(m) as Arc<dyn ChatModel>);
    }
    if !ai.enabled {
        return Err(tr!("ai-error-disabled").into());
    }
    let (mut profile, model) = resolve(ai, route).ok_or_else(|| tr!("ai-error-no-provider").to_string())?;
    profile.model = model;
    if profile.model.trim().is_empty() {
        return Err(tr!("ai-error-no-model-id", name = &profile.name));
    }
    let provider = Provider::from_profile(&profile, NetSettings::from_config(ai)).map_err(|e| e.to_string())?;
    Ok(Arc::new(ProviderModel::new(provider, &profile)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{AiProviderKind, AiTaskModel};

    #[test]
    fn routing_falls_back_to_default_profile() {
        let mut ai = Ai::default();
        assert!(resolve(&ai, Route::Fast).is_none());
        ai.profiles.push(AiProfile { name: "claude".into(), model: "big".into(), ..Default::default() });
        ai.profiles.push(AiProfile {
            name: "local".into(),
            kind: AiProviderKind::Ollama,
            model: "small".into(),
            ..Default::default()
        });
        let (p, m) = resolve(&ai, Route::Strong).unwrap();
        assert_eq!((p.name.as_str(), m.as_str()), ("claude", "big"));
        ai.tasks.fast = AiTaskModel { profile: "local".into(), model: String::new() };
        ai.tasks.strong = AiTaskModel { profile: "missing".into(), model: "override".into() };
        assert_eq!(resolve(&ai, Route::Fast).unwrap().1, "small");
        let (p, m) = resolve(&ai, Route::Strong).unwrap();
        assert_eq!((p.name.as_str(), m.as_str()), ("claude", "override"));
    }
}
