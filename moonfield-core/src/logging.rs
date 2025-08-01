//! Logging initialization and configuration module
//!
//! Provides unified tracing initialization functionality with console and file output support

use tracing::Level;

/// Logging configuration options
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Log level
    pub level: Level,
    /// Enable console output
    pub console_output: bool,
    /// File output path (optional)
    pub file_output: Option<String>,
    /// Enable colored output
    pub colored_output: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            console_output: true,
            file_output: None,
            colored_output: true,
        }
    }
}

/// Initialize tracing logging system
///
/// # Arguments
///
/// * `config` - Logging configuration options
///
/// # Examples
///
/// ```rust
/// use moonfield_core::logging::{init_logging, LoggingConfig};
/// use tracing::Level;
///
/// let config = LoggingConfig {
///     level: Level::DEBUG,
///     console_output: true,
///     file_output: Some("moonfield.log".to_string()),
///     colored_output: true,
/// };
///
/// init_logging(config).expect("Failed to initialize logging");
/// ```
pub fn init_logging(
    config: LoggingConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{
        EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt,
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.level.to_string()));

    if config.console_output {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(false)
                    .with_thread_names(false)
                    .with_file(true)
                    .with_line_number(true)
                    .with_ansi(config.colored_output),
            )
            .init();
    } else {
        tracing_subscriber::registry().with(env_filter).init();
    }

    tracing::info!("Tracing initialized with level: {}", config.level);
    Ok(())
}

/// Initialize logging system with default configuration
pub fn init_default_logging() -> Result<(), Box<dyn std::error::Error>> {
    init_logging(LoggingConfig::default())
}

/// Initialize logging system for development environment (DEBUG level)
pub fn init_dev_logging() -> Result<(), Box<dyn std::error::Error>> {
    init_logging(LoggingConfig {
        level: Level::DEBUG,
        console_output: true,
        file_output: None,
        colored_output: true,
    })
}

/// Automatically select logging configuration based on build type
/// Debug builds use DEBUG level, Release builds use INFO level
pub fn init_auto_logging() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(debug_assertions)]
    {
        init_dev_logging() // Debug build: DEBUG level
    }
    #[cfg(not(debug_assertions))]
    {
        init_default_logging() // Release build: INFO level
    }
}

/// Optimized logging configuration for different build types
pub fn init_optimized_logging() -> Result<(), Box<dyn std::error::Error>> {
    let config = LoggingConfig {
        #[cfg(debug_assertions)]
        level: Level::DEBUG,
        #[cfg(not(debug_assertions))]
        level: Level::INFO,

        console_output: true,

        #[cfg(debug_assertions)]
        colored_output: true, // Debug build: enable colors
        #[cfg(not(debug_assertions))]
        colored_output: false, // Release build: disable colors for performance

        file_output: None,
    };

    init_logging(config)
}
