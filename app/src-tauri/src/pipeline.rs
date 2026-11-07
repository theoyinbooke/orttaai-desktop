//! Which transcript pipeline and voice-edit rewriter the engine uses.
//!
//! The engine area ships the defaults: Personal Memory (dictionary + snippets,
//! honouring the Text settings' toggles and each entry's active flag) and no
//! rewriter. The text / polish / voice-edit areas swap in their concrete
//! implementations here; the engine rebuilds both on `settings-changed` and
//! `memory-changed`, so changes apply without a restart.

use orttaai_core::coordinator::{EditRewriter, MemoryPipeline, TranscriptPipeline};
use orttaai_core::memory::MemoryService;
use orttaai_core::settings::Settings;
use orttaai_core::store::Store;

/// The dictionary + snippets the user has switched on.
pub fn memory_service(settings: &Settings) -> MemoryService {
    let mut memory = MemoryService::new();
    let Ok(store) = Store::open_default() else {
        return memory;
    };
    if settings.dictionary_enabled {
        for e in store.list_dictionary().unwrap_or_default() {
            if e.is_active {
                memory.add_term_with_case(&e.source, e.target, e.is_case_sensitive);
            }
        }
    }
    if settings.snippets_enabled {
        for s in store.list_snippets().unwrap_or_default() {
            if s.is_active {
                memory.add_snippet(&s.trigger, s.expansion);
            }
        }
    }
    memory
}

/// The transcript pipeline for these settings.
pub fn build_pipeline(settings: &Settings) -> Box<dyn TranscriptPipeline> {
    Box::new(MemoryPipeline(memory_service(settings)))
}

/// The voice-edit rewriter for these settings; `None` until a local-LLM
/// rewriter is configured (the engine then explains that edits need one).
pub fn build_rewriter(_settings: &Settings) -> Option<Box<dyn EditRewriter>> {
    None
}
