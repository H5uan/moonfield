//! This crate provides logging functions and configuration for moonfield apps,
//! built on the [`tracing`](https://docs.rs/tracing) ecosystem.
//!
//! The macros provided for logging are re-exported from `tracing`, and behave
//! identically to it.
//!
//! By default, the [`LogPlugin`] sets up a `tracing-subscriber` collector that
//! logs to `stderr`. Log level can be controlled via the `RUST_LOG` environment
//! variable or programmatically through [`LogPlugin`] configuration.

mod once;

/// The log prelude.
///
/// This includes the most common types in this crate, re-exported for your convenience.
pub mod prelude {
    #[doc(hidden)]
    pub use tracing::{
        debug, debug_span, error, error_span, info, info_span, trace, trace_span, warn, warn_span,
    };

    #[doc(hidden)]
    pub use crate::{debug_once, error_once, info_once, once, trace_once, warn_once};
}

pub use crate::once::*;
pub use tracing::{
    self, debug, debug_span, error, error_span, event, info, info_span, trace, trace_span, warn,
    warn_span, Level,
};
pub use tracing_subscriber;

use moonfield_app::{App, Plugin};
use tracing_log::LogTracer;
use tracing_subscriber::{layer::Layered, prelude::*, registry::Registry, EnvFilter, Layer};

/// A boxed [`Layer`] that can be used with [`LogPlugin::custom_layer`].
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

#[cfg(feature = "trace")]
type BaseSubscriber = Layered<EnvFilter, Layered<Option<BoxedLayer>, Registry>>;

#[cfg(feature = "trace")]
type PreFmtSubscriber = Layered<tracing_error::ErrorLayer<BaseSubscriber>, BaseSubscriber>;

#[cfg(not(feature = "trace"))]
type PreFmtSubscriber = Layered<EnvFilter, Layered<Option<BoxedLayer>, Registry>>;

/// A boxed [`Layer`] that can be used to override the default formatter.
pub type BoxedFmtLayer = Box<dyn Layer<PreFmtSubscriber> + Send + Sync + 'static>;

/// The default [`LogPlugin`] [`EnvFilter`].
pub const DEFAULT_FILTER: &str = concat!(
    "wgpu=error,",
    "naga=warn,",
    "calloop::loop_logic=error,",
    "calloop::sources=debug,",
);

/// Adds logging to Apps. This plugin sets up a `tracing-subscriber` collector
/// that logs to `stderr`.
///
/// You can configure this plugin.
/// ```no_run
/// # use moonfield_app::App;
/// # use moonfield_log::LogPlugin;
/// # use tracing::Level;
/// fn main() {
///     App::new()
///         .add_plugins(LogPlugin {
///             level: Level::DEBUG,
///             filter: "wgpu=error,moonfield_render=info".to_string(),
///             custom_layer: |_| None,
///         })
///         .run();
/// }
/// ```
///
/// Log level can also be changed using the `RUST_LOG` environment variable.
/// For example, using `RUST_LOG=wgpu=error,moonfield_render=info cargo run ..`
///
/// If you define the `RUST_LOG` environment variable, the [`LogPlugin`] settings
/// will be ignored.
///
/// To disable color terminal output (ANSI escape codes), set the environment
/// variable `NO_COLOR` to any value. See [no-color.org](https://no-color.org/).
pub struct LogPlugin {
    /// Filters logs using the [`EnvFilter`] format
    pub filter: String,

    /// Filters out logs that are "less than" the given level.
    /// This can be further filtered using the `filter` setting.
    pub level: Level,

    /// Optionally add an extra [`Layer`] to the tracing subscriber.
    ///
    /// This function is only called once, when the plugin is built.
    pub custom_layer: fn(app: &mut App) -> Option<BoxedLayer>,
}

impl Default for LogPlugin {
    fn default() -> Self {
        Self {
            filter: DEFAULT_FILTER.to_string(),
            level: Level::INFO,
            custom_layer: |_| None,
        }
    }
}

impl Plugin for LogPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "trace")]
        {
            let old_handler = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |infos| {
                eprintln!("{}", tracing_error::SpanTrace::capture());
                old_handler(infos);
            }));
        }

        let subscriber = Registry::default();

        // add optional layer provided by user
        let subscriber = subscriber.with((self.custom_layer)(app));

        let subscriber = subscriber.with(self.build_filter_layer());

        #[cfg(feature = "trace")]
        let subscriber = subscriber.with(tracing_error::ErrorLayer::default());

        let fmt_layer = (self.fmt_layer())(app).unwrap_or_else(|| {
            Box::new(tracing_subscriber::fmt::Layer::default().with_writer(std::io::stderr))
        });

        let subscriber = subscriber.with(fmt_layer);

        let logger_already_set = LogTracer::init().is_err();
        let subscriber_already_set = tracing::subscriber::set_global_default(subscriber).is_err();

        match (logger_already_set, subscriber_already_set) {
            (true, true) => tracing::error!(
                "Could not set global logger and tracing subscriber as they are already set. Consider disabling LogPlugin."
            ),
            (true, false) => tracing::error!(
                "Could not set global logger as it is already set. Consider disabling LogPlugin."
            ),
            (false, true) => tracing::error!(
                "Could not set global tracing subscriber as it is already set. Consider disabling LogPlugin."
            ),
            (false, false) => (),
        }
    }
}

impl LogPlugin {
    fn build_filter_layer(&self) -> EnvFilter {
        // Start with the default filters, then add the env filters afterwards, so that the env filters
        // can be used to selectively override the default filters
        let default_filters =
            EnvFilter::builder().parse_lossy(format!("{},{}", self.level, self.filter));
        // We must manually parse and add the directives individually because `EnvFilter` has no helper methods for adding
        // multiple directives at once.
        let env_filters = std::env::var(EnvFilter::DEFAULT_ENV).unwrap_or_default();
        let result = env_filters
            .split(',')
            .filter(|s| !s.is_empty())
            .try_fold(default_filters.clone(), |filters, directive| {
                directive.parse().map(|d| filters.add_directive(d))
            });
        // Fall back to just the default filters if the env filters are malformed
        match result {
            Ok(combined_filters) => combined_filters,
            Err(e) => {
                eprintln!("LogPlugin failed to parse filter from env: {e}");
                default_filters
            }
        }
    }

    /// Override the default [`tracing_subscriber::fmt::Layer`] with a custom one.
    ///
    /// This allows you to overwrite the default formatter layer, for example
    /// using [`tracing_subscriber::fmt::Layer::without_time`] to remove the
    /// timestamp from the log output.
    fn fmt_layer(&self) -> fn(&mut App) -> Option<BoxedFmtLayer> {
        |_| None
    }
}

/// Call [`trace!`](crate::trace) once per call site.
///
/// Useful for logging within systems which are called every frame.
#[macro_export]
macro_rules! trace_once {
    ($($arg:tt)+) => ({
        $crate::once!($crate::trace!($($arg)+))
    });
}

/// Call [`debug!`](crate::debug) once per call site.
///
/// Useful for logging within systems which are called every frame.
#[macro_export]
macro_rules! debug_once {
    ($($arg:tt)+) => ({
        $crate::once!($crate::debug!($($arg)+))
    });
}

/// Call [`info!`](crate::info) once per call site.
///
/// Useful for logging within systems which are called every frame.
#[macro_export]
macro_rules! info_once {
    ($($arg:tt)+) => ({
        $crate::once!($crate::info!($($arg)+))
    });
}

/// Call [`warn!`](crate::warn) once per call site.
///
/// Useful for logging within systems which are called every frame.
#[macro_export]
macro_rules! warn_once {
    ($($arg:tt)+) => ({
        $crate::once!($crate::warn!($($arg)+))
    });
}

/// Call [`error!`](crate::error) once per call site.
///
/// Useful for logging within systems which are called every frame.
#[macro_export]
macro_rules! error_once {
    ($($arg:tt)+) => ({
        $crate::once!($crate::error!($($arg)+))
    });
}
