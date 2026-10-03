//! Service handles that follow the published index generation.

use bg_spec_core::{
    config::Config,
    services::{ServiceSettings, Services},
};
use std::sync::{Arc, RwLock};

/// Supplies the services a tool call runs against.
///
/// A reloading source checks the data directory's `CURRENT` file on every call (a tiny read) and,
/// when the indexer has published a new generation, opens it and swaps the handles. In-flight calls
/// keep the services they started with, and a generation that fails to open is logged and skipped,
/// so the server keeps answering from the last good generation.
#[derive(Clone)]
pub struct ServiceSource(Inner);

#[derive(Clone)]
enum Inner {
    Fixed(Services),
    Reloading(Arc<Reloader>),
}

struct Reloader {
    config: Config,
    current: RwLock<Loaded>,
}

#[derive(Clone)]
struct Loaded {
    generation: Option<u64>,
    services: Services,
}

impl ServiceSource {
    /// Always serves `services` (tests and embedded use).
    pub fn fixed(services: Services) -> Self {
        Self(Inner::Fixed(services))
    }

    /// Opens the published generation and follows later publications.
    pub async fn open(config: Config) -> bg_spec_core::Result<Self> {
        let loaded = load(&config).await?;
        Ok(Self(Inner::Reloading(Arc::new(Reloader {
            config,
            current: RwLock::new(loaded),
        }))))
    }

    /// Services of the newest generation that could be opened.
    pub async fn services(&self) -> Services {
        self.loaded().await.services
    }

    /// Published generation currently served (`None` for fixed services or the legacy layout).
    pub async fn generation(&self) -> Option<u64> {
        self.loaded().await.generation
    }

    async fn loaded(&self) -> Loaded {
        match &self.0 {
            Inner::Fixed(services) => Loaded {
                generation: None,
                services: services.clone(),
            },
            Inner::Reloading(r) => r.loaded().await,
        }
    }
}

impl Reloader {
    fn snapshot(&self) -> Loaded {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    async fn loaded(&self) -> Loaded {
        let current = self.snapshot();
        let published = match bg_spec_store::layout::read_current(&self.config.data_dir) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "cannot read the published index generation");
                return current;
            }
        };
        if published.is_none() || published == current.generation {
            return current;
        }
        match load(&self.config).await {
            Ok(next) => {
                tracing::info!(
                    from = ?current.generation,
                    to = ?next.generation,
                    "switched to newly published index generation"
                );
                let mut slot = self.current.write().unwrap_or_else(|e| e.into_inner());
                // A concurrent call may already have loaded an equal or newer generation.
                if slot.generation < next.generation {
                    *slot = next;
                }
                slot.clone()
            }
            Err(e) => {
                tracing::warn!(
                    generation = ?published,
                    error = %e,
                    "cannot open published index generation; still serving the previous one"
                );
                current
            }
        }
    }
}

async fn load(config: &Config) -> bg_spec_core::Result<Loaded> {
    let (catalog, search, layout) = bg_spec_store::open_current(config).await?;
    Ok(Loaded {
        generation: layout.generation,
        services: Services::new(catalog, search, ServiceSettings::from_config(config)),
    })
}
