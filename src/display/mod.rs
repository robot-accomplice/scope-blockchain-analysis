//! # Display Module
//!
//! This module provides display utilities for terminal output and report generation.
//!
//! ## Features
//!
//! - **ASCII Charts**: Line charts, bar charts, and distribution visualizations
//! - **Markdown Reports**: Comprehensive token analysis reports
//!
//! ## Usage
//!
//! ```rust,no_run
//! use bcc::display::charts::{render_price_chart, render_volume_chart};
//! use bcc::chains::PricePoint;
//!
//! let price_history = vec![
//!     PricePoint { timestamp: 0, price: 1.0 },
//!     PricePoint { timestamp: 3600, price: 1.05 },
//!     PricePoint { timestamp: 7200, price: 1.02 },
//! ];
//!
//! let chart = render_price_chart(&price_history, 60, 10);
//! println!("{}", chart);
//! ```

pub mod charts;
pub mod compliance;
pub mod report;

pub use charts::{
    ChartConfig, render_holder_distribution, render_price_chart, render_volume_chart,
};
pub use compliance::{format_risk_report, OutputFormat};
pub use report::{generate_report, save_report};
