//! SPIKE (serde-yaml-replacement, not for landing): the one YAML crate this crate's typed
//! reads and writes go through.
//!
//! Without the `yaml-spike` feature this is `serde_norway`, exactly the calls the call sites
//! made before. With it, each call goes through whichever backend [`select`] named — or, when
//! nothing selected one, `serde-saphyr` unless `ONETASKGRAPH_YAML_SPIKE=norway` — so the
//! differential harness under `spikes/saphyr-diff` can drive the real code paths through both,
//! and a binary built with the feature runs every journey on `serde-saphyr`.
//!
//! (The template front-matter walk lives in `onetaskgraph-core` and is measured by the harness.)

#[cfg(not(feature = "yaml-spike"))]
pub(crate) use serde_norway::{from_str, to_string};

#[cfg(feature = "yaml-spike")]
pub use switch::*;

#[cfg(feature = "yaml-spike")]
mod switch {
    use std::fmt;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU8, Ordering};

    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Which crate a call goes through.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Backend {
        /// `serde_norway`, what this crate uses today.
        Norway,
        /// `serde-saphyr`, the candidate.
        Saphyr,
    }

    static SELECTED: AtomicU8 = AtomicU8::new(0);
    static DE_OPTIONS: Mutex<Option<fn() -> serde_saphyr::Options>> = Mutex::new(None);
    static SER_OPTIONS: Mutex<Option<fn() -> serde_saphyr::SerializerOptions>> =
        Mutex::new(None);

    /// Send every later call through `backend`, process-wide.
    pub fn select(backend: Backend) {
        SELECTED.store(
            match backend {
                Backend::Norway => 1,
                Backend::Saphyr => 2,
            },
            Ordering::SeqCst,
        );
    }

    /// The `serde-saphyr` options every later read and write uses; `None` for its defaults.
    pub fn saphyr_options(
        de: Option<fn() -> serde_saphyr::Options>,
        ser: Option<fn() -> serde_saphyr::SerializerOptions>,
    ) {
        *DE_OPTIONS.lock().expect("unpoisoned") = de;
        *SER_OPTIONS.lock().expect("unpoisoned") = ser;
    }

    /// The backend a call made now goes through.
    #[must_use]
    pub fn selected() -> Backend {
        match SELECTED.load(Ordering::SeqCst) {
            1 => Backend::Norway,
            2 => Backend::Saphyr,
            _ if std::env::var("ONETASKGRAPH_YAML_SPIKE").as_deref() == Ok("norway") => {
                Backend::Norway
            }
            _ => Backend::Saphyr,
        }
    }

    /// Deserializer options chosen to reproduce `serde_norway` where an option can — the
    /// harness's `compat` profile, and what a build with `yaml-spike-compat` reads with.
    #[must_use]
    pub fn compat_de() -> serde_saphyr::Options {
        serde_saphyr::options! {
            // `yes`/`no`/`on`/`off`/`y`/`n` stay strings, as YAML 1.2 and serde_norway read
            // them — and as serde_norway writes them, unquoted.
            strict_booleans: true,
            // `<<` is an ordinary key, as serde_norway reads it.
            merge_keys: serde_saphyr::MergeKeyPolicy::AsOrdinary,
            // A repeated key: the later value, as serde_norway gives a map. (A struct's
            // repeated field too, which serde_norway refuses: no one policy does both.)
            duplicate_keys: serde_saphyr::DuplicateKeyPolicy::LastWins,
            // The error alone, on one line, as serde_norway's — no source snippet.
            with_snippet: false,
        }
    }

    /// Serializer options chosen to reproduce `serde_norway` where an option can.
    #[must_use]
    pub fn compat_ser() -> serde_saphyr::SerializerOptions {
        serde_saphyr::ser_options! {
            // Never fold a long single-line string into `>-`, as serde_norway never does.
            folded_wrap_chars: usize::MAX,
        }
    }

    /// The options a call uses when nothing selected any: the compat profile in a build with
    /// `yaml-spike-compat`, else serde-saphyr's defaults.
    fn fallback_de() -> Option<fn() -> serde_saphyr::Options> {
        cfg!(feature = "yaml-spike-compat").then_some(compat_de as fn() -> serde_saphyr::Options)
    }

    fn fallback_ser() -> Option<fn() -> serde_saphyr::SerializerOptions> {
        cfg!(feature = "yaml-spike-compat")
            .then_some(compat_ser as fn() -> serde_saphyr::SerializerOptions)
    }

    /// Either crate's error, as the text it displays.
    #[derive(Debug)]
    pub struct Error(String);

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for Error {}

    /// `serde_norway::from_str` or `serde_saphyr::from_str`.
    ///
    /// # Errors
    ///
    /// The selected crate's refusal.
    pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
        match selected() {
            Backend::Norway => serde_norway::from_str(text).map_err(|e| Error(e.to_string())),
            Backend::Saphyr => {
                let options = DE_OPTIONS.lock().expect("unpoisoned").or_else(fallback_de);
                match options {
                    Some(options) => serde_saphyr::from_str_with_options(text, options()),
                    None => serde_saphyr::from_str(text),
                }
                .map_err(|e| Error(e.to_string()))
            }
        }
    }

    /// `serde_norway::to_string` or `serde_saphyr::to_string`.
    ///
    /// # Errors
    ///
    /// The selected crate's refusal.
    pub fn to_string<T: Serialize + ?Sized>(value: &T) -> Result<String, Error> {
        match selected() {
            Backend::Norway => serde_norway::to_string(value).map_err(|e| Error(e.to_string())),
            Backend::Saphyr => {
                let options = SER_OPTIONS.lock().expect("unpoisoned").or_else(fallback_ser);
                match options {
                    Some(options) => serde_saphyr::to_string_with_options(&value, options()),
                    None => serde_saphyr::to_string(&value),
                }
                .map_err(|e| Error(e.to_string()))
            }
        }
    }
}
