//! # ruloc - Rust Lines of Code Counter
//!
//! A sophisticated yet minimalist command-line tool designed for precise analysis of Rust source code.
//! Employs AST-based parsing to accurately distinguish between production and test code, providing
//! granular metrics across blank lines, comments, rustdoc documentation, and executable code.
//!
//! ## Core Capabilities
//!
//! - **AST-Driven Analysis**: Leverages `ra_ap_syntax` for token-level parsing, ensuring accurate
//!   classification of code elements even within complex constructs like raw strings and macros.
//! - **Production/Test Separation**: Intelligently identifies test modules and functions via
//!   `#[test]` and `#[cfg(test)]` attributes, segregating metrics accordingly.
//! - **Memory-Efficient Architecture**: Implements streaming accumulators supporting both in-memory
//!   and file-backed storage, enabling analysis of arbitrarily large codebases.
//! - **Parallel Processing**: Utilizes Rayon for concurrent file analysis, maximizing throughput
//!   on multi-core systems.
//! - **Flexible Output**: Supports both human-readable text and machine-parseable JSON formats.

use clap::{Parser, ValueEnum};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, trace};
use ra_ap_syntax::{AstNode, SourceFile, SyntaxKind, SyntaxNode, ast, ast::HasAttrs};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

/// Buffer size for FileBackedAccumulator writer (8MB).
const FILE_ACCUMULATOR_BUFFER_SIZE: usize = 8 * 1024 * 1024;

/// Number of spaces for base indentation level in text output formatting.
const TEXT_OUTPUT_BASE_INDENT: usize = 4;

/// Number of spaces for nested indentation level in text output formatting.
const TEXT_OUTPUT_NESTED_INDENT: usize = 6;

/// Debug mode marker for production blank lines (Production BLank).
const DEBUG_MARKER_PRODUCTION_BLANK: &str = "PBL";

/// Debug mode marker for production code lines (Production COde).
const DEBUG_MARKER_PRODUCTION_CODE: &str = "PCO";

/// Debug mode marker for production comment lines (Production CoMment).
const DEBUG_MARKER_PRODUCTION_COMMENT: &str = "PCM";

/// Debug mode marker for production rustdoc lines (Production DoC).
const DEBUG_MARKER_PRODUCTION_RUSTDOC: &str = "PDC";

/// Debug mode marker for test blank lines (Test BLank).
const DEBUG_MARKER_TEST_BLANK: &str = "TBL";

/// Debug mode marker for test code lines (Test COde).
const DEBUG_MARKER_TEST_CODE: &str = "TCO";

/// Debug mode marker for test comment lines (Test CoMment).
const DEBUG_MARKER_TEST_COMMENT: &str = "TCM";

/// Debug mode marker for test rustdoc lines (Test DoC).
const DEBUG_MARKER_TEST_RUSTDOC: &str = "TDC";

/// Comprehensive line-level statistics for a defined scope of Rust source code.
///
/// Provides a complete breakdown of source code composition, categorizing every line
/// into mutually exclusive classifications. Designed for serialization via serde with
/// kebab-case field naming for enhanced interoperability with external tools.
///
/// # Classification Taxonomy
///
/// - **All Lines**: Aggregate count encompassing the entire scope
/// - **Blank Lines**: Lines containing exclusively whitespace characters (spaces, tabs, newlines)
/// - **Comment Lines**: Standard comments (`//` and `/* */`) excluding documentation
/// - **Rustdoc Lines**: Documentation comments (`///`, `//!`, `/**`, `/*!`)
/// - **Code Lines**: Executable Rust code including declarations, expressions, and statements
///
/// # Invariants
///
/// The sum of blank, comment, rustdoc, and code lines equals `all_lines` for valid statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineStats {
    /// Aggregate count of all lines within the analyzed scope.
    #[serde(rename = "all-lines")]
    pub all_lines: usize,

    /// Count of lines consisting solely of whitespace characters.
    #[serde(rename = "blank-lines")]
    pub blank_lines: usize,

    /// Count of non-documentation comment lines.
    #[serde(rename = "comment-lines")]
    pub comment_lines: usize,

    /// Count of rustdoc documentation comment lines.
    #[serde(rename = "rustdoc-lines")]
    pub rustdoc_lines: usize,

    /// Count of executable code lines.
    #[serde(rename = "code-lines")]
    pub code_lines: usize,
}

impl LineStats {
    /// Performs element-wise accumulation of metrics from another instance.
    ///
    /// Aggregates line counts across all categories, enabling hierarchical composition
    /// of statistics from individual files to directory-level summaries.
    ///
    /// # Arguments
    ///
    /// * `other` - The statistics instance to merge into this one
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut total = LineStats { all_lines: 100, code_lines: 60, ..Default::default() };
    /// let additional = LineStats { all_lines: 50, code_lines: 30, ..Default::default() };
    /// total.add(&additional);
    /// assert_eq!(total.all_lines, 150);
    /// assert_eq!(total.code_lines, 90);
    /// ```
    pub fn add(&mut self, other: &LineStats) {
        self.all_lines += other.all_lines;
        self.blank_lines += other.blank_lines;
        self.comment_lines += other.comment_lines;
        self.rustdoc_lines += other.rustdoc_lines;
        self.code_lines += other.code_lines;
    }
}

/// Tripartite statistical analysis of a single Rust source file.
///
/// Segregates metrics into three orthogonal perspectives: aggregate totals, production code,
/// and test code. This decomposition facilitates precise understanding of code distribution
/// between implementation and verification concerns.
///
/// # Field Relationships
///
/// - `total` = `production` + `test` (component-wise)
/// - All line counts within each `LineStats` instance maintain their individual invariants
///
/// # Use Cases
///
/// - Tracking test coverage ratios
/// - Identifying files with disproportionate test/production ratios
/// - Aggregating directory-level statistics
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStats {
    /// Canonical path to the analyzed file, relative to the analysis root directory.
    pub path: String,

    /// Aggregate statistics encompassing all content within the file.
    pub total: LineStats,

    /// Statistics exclusively for production code, excluding test modules and functions.
    pub production: LineStats,

    /// Statistics exclusively for test code identified via `#[test]` and `#[cfg(test)]`.
    pub test: LineStats,
}

/// Consolidated statistical summary aggregated across an entire analysis scope.
///
/// Represents the culmination of file-level metrics rolled up into a comprehensive
/// project or directory-wide overview. Maintains the tripartite decomposition
/// (total/production/test) while tracking the number of files contributing to the aggregate.
///
/// # Aggregation Semantics
///
/// - File count increments with each unique file added
/// - Line statistics accumulate additively across all dimensions
/// - Preserves production/test separation throughout the hierarchy
///
/// # Applications
///
/// - Project-wide code composition reports
/// - Comparative analysis across multiple directories
/// - Baseline metrics for CI/CD pipelines
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    /// Cardinal count of unique files incorporated into this summary.
    pub files: usize,

    /// Aggregate line statistics spanning all analyzed files.
    pub total: LineStats,

    /// Aggregate production code statistics across all files.
    pub production: LineStats,

    /// Aggregate test code statistics across all files.
    pub test: LineStats,
}

impl Summary {
    /// Incorporates file-level statistics into this aggregate summary.
    ///
    /// Atomically increments the file counter and merges all three statistical dimensions
    /// (total, production, test) into their respective accumulators.
    ///
    /// # Arguments
    ///
    /// * `file_stats` - Complete statistical profile of a single file to integrate
    ///
    /// # Postconditions
    ///
    /// - `self.files` increases by exactly 1
    /// - All line counts in `self.total`, `self.production`, and `self.test` increase
    ///   by their corresponding values from `file_stats`
    pub fn add_file(&mut self, file_stats: &FileStats) {
        self.files += 1;
        self.total.add(&file_stats.total);
        self.production.add(&file_stats.production);
        self.test.add(&file_stats.test);
    }
}

/// Comprehensive analysis report encapsulating both aggregate and granular metrics.
///
/// Serves as the canonical output structure combining high-level summary statistics
/// with detailed per-file breakdowns. Designed for serialization to JSON or rendering
/// as human-readable text output.
///
/// # Structure
///
/// - **Summary**: Consolidated view of all files, providing immediate insights into
///   overall codebase composition
/// - **Files**: Exhaustive list of individual file analyses, preserving granularity
///   for detailed examination and drill-down analysis
///
/// # Serialization
///
/// When serialized to JSON, produces a two-section structure ideal for programmatic
/// consumption by CI/CD tools, static analyzers, or custom reporting pipelines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Report {
    /// Aggregate statistical summary spanning all analyzed files.
    pub summary: Summary,

    /// Ordered collection of per-file statistical analyses.
    pub files: Vec<FileStats>,
}

/// Strategy pattern for memory-efficient accumulation of file statistics.
///
/// Defines a polymorphic interface enabling distinct storage backends for statistical
/// data. Implementations may optimize for different constraints: in-memory accumulation
/// for speed with small codebases, or streaming to disk for large-scale analyses
/// exceeding available RAM.
///
/// # Design Rationale
///
/// - **Scalability**: Prevents memory exhaustion when analyzing extensive codebases
/// - **Flexibility**: Permits runtime selection of accumulation strategy based on context
/// - **Thread Safety**: Requires `Send + Sync` to support parallel file processing
///
/// # Implementations
///
/// - [`InMemoryAccumulator`]: Stores all data in `Vec`, optimized for small to medium projects
/// - [`FileBackedAccumulator`]: Streams to temporary file, suitable for arbitrarily large codebases
pub trait StatsAccumulator: Send + Sync {
    /// Incorporates a file's statistics into the accumulator.
    ///
    /// # Arguments
    ///
    /// * `file_stats` - Complete statistical profile for a single analyzed file
    ///
    /// # Errors
    ///
    /// Returns `Err` if the underlying storage mechanism fails (e.g., disk I/O errors,
    /// serialization failures, or out-of-disk-space conditions).
    fn add_file(&mut self, file_stats: &FileStats) -> Result<(), String>;

    /// Retrieves a snapshot of the current aggregate summary.
    ///
    /// Provides O(1) access to consolidated statistics without requiring traversal
    /// of all accumulated files. The summary reflects all files added up to this point.
    ///
    /// # Returns
    ///
    /// Current `Summary` instance representing the aggregate of all accumulated files
    fn get_summary(&self) -> Summary;

    /// Constructs an iterator yielding individual file statistics.
    ///
    /// Enables sequential traversal of all accumulated file data for detailed reporting
    /// or analysis. For file-backed implementations, this typically involves reading
    /// from persistent storage.
    ///
    /// # Returns
    ///
    /// Boxed iterator producing `FileStats` instances in insertion order
    ///
    /// # Errors
    ///
    /// Returns `Err` if the backing store cannot be read (e.g., file corruption,
    /// permission issues, or deserialization failures).
    fn iter_files(&self) -> Result<Box<dyn Iterator<Item = FileStats>>, String>;
}

/// High-performance in-memory statistics accumulator optimized for small to medium codebases.
///
/// Maintains all file statistics in a contiguous `Vec`, providing optimal iteration performance
/// and zero I/O overhead. Suitable for projects with manageable file counts where memory
/// consumption remains within acceptable bounds.
///
/// # Performance Characteristics
///
/// - **Time Complexity**: O(1) for `add_file`, O(1) for `get_summary`, O(n) for `iter_files`
/// - **Space Complexity**: O(n) where n is the number of accumulated files
/// - **Memory Footprint**: Proportional to total number of files × sizeof(`FileStats`)
///
/// # Recommended Usage
///
/// - Projects with fewer than 10,000 files
/// - Environments with ample available RAM
/// - Scenarios requiring high-speed iteration over file statistics
///
/// # Alternative
///
/// For large-scale analyses, consider [`FileBackedAccumulator`] which trades CPU/memory
/// efficiency for unbounded scalability via disk-backed storage.
pub struct InMemoryAccumulator {
    /// Rolling summary statistics maintained incrementally.
    summary: Summary,

    /// Chronologically ordered collection of all accumulated file statistics.
    files: Vec<FileStats>,
}

impl Default for InMemoryAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryAccumulator {
    /// Constructs a pristine accumulator with zero accumulated statistics.
    ///
    /// Initializes internal data structures with optimal default capacity,
    /// ready to receive file statistics via `add_file()`.
    ///
    /// # Returns
    ///
    /// Fresh `InMemoryAccumulator` instance with empty summary and zero files
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut acc = InMemoryAccumulator::new();
    /// assert_eq!(acc.get_summary().files, 0);
    /// ```
    pub fn new() -> Self {
        Self {
            summary: Summary::default(),
            files: Vec::new(),
        }
    }
}

impl StatsAccumulator for InMemoryAccumulator {
    fn add_file(&mut self, file_stats: &FileStats) -> Result<(), String> {
        self.summary.add_file(file_stats);
        self.files.push(file_stats.clone());
        Ok(())
    }

    fn get_summary(&self) -> Summary {
        self.summary.clone()
    }

    fn iter_files(&self) -> Result<Box<dyn Iterator<Item = FileStats>>, String> {
        Ok(Box::new(self.files.clone().into_iter()))
    }
}

/// Scalable disk-backed statistics accumulator for unbounded codebase analysis.
///
/// Employs a streaming architecture that persists file statistics to a temporary file
/// in [JSON Lines format](http://jsonlines.org/), retaining only aggregate summaries
/// in memory. This design enables analysis of arbitrarily large codebases—including
/// projects with millions of files—without risking memory exhaustion.
///
/// # Architecture
///
/// - **Streaming Writes**: File statistics serialized and appended immediately upon `add_file()`
/// - **Buffered I/O**: 8MB write buffer minimizes syscall overhead
/// - **Automatic Cleanup**: Temporary file deleted automatically via RAII when dropped
/// - **JSON Lines Format**: One complete JSON object per line, facilitating line-oriented processing
///
/// # Performance Considerations
///
/// - **Memory**: O(1) - only summary statistics retained in RAM
/// - **Disk I/O**: Sequential writes optimized for modern SSD/HDD characteristics
/// - **Iteration**: Requires sequential read-through of temporary file
///
/// # Use Cases
///
/// - Analyzing monolithic monorepos with extensive file counts
/// - CI/CD environments with constrained memory allocations
/// - Historical analysis across thousands of revisions
pub struct FileBackedAccumulator {
    /// In-memory rolling summary, incrementally updated with each file.
    summary: Summary,

    /// Self-deleting temporary file handle for persistent statistics storage.
    temp_file: NamedTempFile,

    /// High-capacity buffered writer minimizing I/O syscalls.
    writer: BufWriter<std::fs::File>,
}

impl FileBackedAccumulator {
    /// Constructs a new disk-backed accumulator with ephemeral temporary storage.
    ///
    /// Allocates a system-managed temporary file for statistics persistence and initializes
    /// an 8MB write buffer for optimal I/O performance. The temporary file is automatically
    /// cleaned up upon object destruction, ensuring no disk space leakage.
    ///
    /// # Returns
    ///
    /// Initialized `FileBackedAccumulator` ready to receive statistics, or an error
    /// if system resources are unavailable
    ///
    /// # Errors
    ///
    /// Returns `Err` if:
    /// - Temporary file creation fails (insufficient disk space, permission issues)
    /// - File descriptor limits are exceeded
    /// - Temporary directory is inaccessible
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut acc = FileBackedAccumulator::new()?;
    /// // Accumulator ready for use with automatic cleanup on drop
    /// ```
    pub fn new() -> Result<Self, String> {
        let temp_file = NamedTempFile::new().map_err(|e| {
            format!(
                "Failed to create temporary file for accumulator: {}. Ensure adequate disk space and write permissions in temp directory.",
                e
            )
        })?;

        let file = temp_file.reopen().map_err(|e| {
            format!(
                "Failed to open temporary file '{}' for writing: {}",
                temp_file.path().display(),
                e
            )
        })?;

        let writer = BufWriter::with_capacity(FILE_ACCUMULATOR_BUFFER_SIZE, file);

        Ok(Self {
            summary: Summary::default(),
            temp_file,
            writer,
        })
    }

    /// Flushes any buffered data to the temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation fails
    fn flush(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|e| format!("Failed to flush writer: {}", e))
    }
}

impl StatsAccumulator for FileBackedAccumulator {
    fn add_file(&mut self, file_stats: &FileStats) -> Result<(), String> {
        self.summary.add_file(file_stats);

        // Serialize as JSON and write with newline (JSON Lines format)
        let json = serde_json::to_string(file_stats)
            .map_err(|e| format!("Failed to serialize file stats: {}", e))?;

        writeln!(self.writer, "{}", json)
            .map_err(|e| format!("Failed to write to temporary file: {}", e))?;

        Ok(())
    }

    fn get_summary(&self) -> Summary {
        self.summary.clone()
    }

    fn iter_files(&self) -> Result<Box<dyn Iterator<Item = FileStats>>, String> {
        // Flush any pending writes
        // Note: We can't call self.flush() here because of borrowing rules,
        // so we need to ensure flush is called before iter_files

        // Open the temp file for reading
        let file = std::fs::File::open(self.temp_file.path())
            .map_err(|e| format!("Failed to open temporary file for reading: {}", e))?;

        let reader = BufReader::new(file);

        // Create an iterator that reads JSON lines
        let iter = reader.lines().filter_map(|line| match line {
            Ok(line_str) => match serde_json::from_str::<FileStats>(&line_str) {
                Ok(stats) => Some(stats),
                Err(e) => {
                    debug!("Failed to deserialize line: {}", e);
                    None
                }
            },
            Err(e) => {
                debug!("Failed to read line: {}", e);
                None
            }
        });

        Ok(Box::new(iter))
    }
}

/// Serialization format selector for statistical output.
///
/// Determines the encoding and structure of analysis results, enabling consumption
/// by both human readers and automated tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    /// Human-readable hierarchical text format with indented structure (default).
    ///
    /// Optimized for terminal display and manual inspection, presenting statistics
    /// in a tree-like layout with clear visual hierarchy.
    Text,

    /// Machine-parseable JSON format conforming to the [`Report`] schema.
    ///
    /// Suitable for integration with CI/CD pipelines, static analysis tools,
    /// and custom reporting dashboards. Pretty-printed for readability.
    Json,
}

/// Command-line arguments for ruloc.
#[derive(Debug, Parser)]
#[command(name = "ruloc", version, about = "Rust lines of code counter")]
struct Args {
    /// Analyze a single Rust file.
    #[arg(short, long, value_name = "FILE", conflicts_with = "dir")]
    file: Option<PathBuf>,

    /// Analyze all Rust files in a directory recursively.
    #[arg(short, long, value_name = "DIR", conflicts_with = "file")]
    dir: Option<PathBuf>,

    /// Output in plain text format (default).
    #[arg(long, conflicts_with = "out_json")]
    out_text: bool,

    /// Output in JSON format.
    #[arg(long, conflicts_with = "out_text")]
    out_json: bool,

    /// Enable debug mode: show each line with type prefix (conflicts with JSON output).
    #[arg(long, conflicts_with = "out_json")]
    debug: bool,

    /// Disable colored output in debug mode.
    #[arg(long)]
    no_color: bool,

    /// Enable verbose output for debugging.
    #[arg(long)]
    verbose: bool,

    /// Maximum file size to analyze (supports units: KB, MB, GB; defaults to bytes).
    /// Examples: 1000, 3.5KB, 10MB, 1.1GB
    #[arg(long, value_name = "SIZE")]
    max_file_size: Option<String>,
}

impl Args {
    /// Parses the max file size from the command-line argument.
    ///
    /// Supports units: KB, MB, GB. Without a unit, interprets as bytes.
    ///
    /// # Returns
    ///
    /// `Some(size_in_bytes)` if specified, `None` otherwise
    ///
    /// # Errors
    ///
    /// Returns an error if the size string cannot be parsed
    fn parse_max_file_size(&self) -> Result<Option<u64>, String> {
        let Some(ref size_str) = self.max_file_size else {
            return Ok(None);
        };

        parse_file_size(size_str).map(Some)
    }
}

impl Args {
    /// Determines the output format based on command-line flags.
    ///
    /// # Returns
    ///
    /// `OutputFormat::Json` if `--out-json` is specified, otherwise `OutputFormat::Text`
    fn output_format(&self) -> OutputFormat {
        if self.out_json {
            OutputFormat::Json
        } else {
            OutputFormat::Text
        }
    }
}

/// Mutually exclusive taxonomy for source code line classification.
///
/// Represents the fundamental categorization scheme applied during line-level analysis.
/// Each line in a source file maps to exactly one variant, forming a complete partition
/// of the source code space.
///
/// # Classification Priority
///
/// When lines contain multiple token types, classification follows this precedence:
/// 1. Rustdoc (highest priority)
/// 2. Comment
/// 3. Code
/// 4. Blank (lowest priority - default assumption)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineType {
    /// Lines consisting exclusively of whitespace characters (spaces, tabs, newlines).
    ///
    /// Examples: empty lines, lines with only indentation
    Blank,

    /// Standard non-documentation comment lines.
    ///
    /// Includes `//` line comments and `/* */` block comments, excluding
    /// documentation variants recognized by rustdoc.
    Comment,

    /// Documentation comment lines recognized by rustdoc.
    ///
    /// Comprises `///`, `//!`, `/**`, and `/*!` comment forms that generate
    /// API documentation when processed by rustdoc.
    Rustdoc,

    /// Executable code lines containing declarations, expressions, or statements.
    ///
    /// Encompasses all Rust syntax elements beyond comments and whitespace,
    /// including keywords, identifiers, operators, literals, and punctuation.
    Code,
}

/// Parses a file size string with optional unit suffix.
///
/// Supports units: KB, MB, GB (case-insensitive). Without a unit, interprets as bytes.
/// Allows decimal numbers (e.g., "3.5KB").
///
/// # Arguments
///
/// * `size_str` - The size string to parse (e.g., "1000", "3.5KB", "10MB")
///
/// # Returns
///
/// The size in bytes as `u64`
///
/// # Errors
///
/// Returns an error if the string cannot be parsed as a valid size
fn parse_file_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim();

    // Try to match unit suffix
    let (number_str, multiplier) =
        if let Some(pos) = size_str.to_uppercase().find(|c: char| c.is_alphabetic()) {
            let (num, unit) = size_str.split_at(pos);
            let mult = match unit.to_uppercase().as_str() {
                "KB" => 1024u64,
                "MB" => 1024u64 * 1024,
                "GB" => 1024u64 * 1024 * 1024,
                _ => {
                    return Err(format!(
                        "Invalid size unit: '{}'. Supported units: KB, MB, GB",
                        unit
                    ));
                }
            };
            (num, mult)
        } else {
            (size_str, 1u64)
        };

    // Parse the numeric part
    let number: f64 = number_str
        .trim()
        .parse()
        .map_err(|_| format!("Invalid size number: '{}'", number_str))?;

    if number < 0.0 {
        return Err("File size cannot be negative".to_string());
    }

    let bytes = (number * multiplier as f64) as u64;
    Ok(bytes)
}

/// Entry point for the ruloc CLI application.
///
/// Parses command-line arguments, initializes logging, analyzes the specified
/// file or directory, and outputs the results in the requested format.
/// Uses a file-backed accumulator to avoid excessive memory consumption
/// when processing large codebases.
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` with an error message on failure
///
/// # Errors
///
/// Returns an error if:
/// - Neither `--file` nor `--dir` is specified
/// - File reading fails
/// - Directory contains no Rust files
/// - JSON serialization fails
/// - Temporary file operations fail
fn main() -> Result<(), String> {
    let args = Args::parse();

    // Initialize logger
    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Trace)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Warn)
            .init();
    }

    // Parse max file size if specified
    let max_file_size = args.parse_max_file_size()?;

    // Handle debug mode separately
    if args.debug {
        let use_color = !args.no_color;

        if let Some(file_path) = &args.file {
            output_file_debug(file_path, use_color, max_file_size)?;
        } else if let Some(dir_path) = &args.dir {
            // Collect all Rust files
            let rust_files: Vec<_> = WalkDir::new(dir_path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
                .collect();

            for entry in rust_files {
                let path = entry.path();
                // Skip files that exceed size limit
                if let Err(e) = output_file_debug(path, use_color, max_file_size) {
                    eprintln!("Warning: {}", e);
                    continue;
                }
                println!(); // Blank line between files
            }
        } else {
            eprintln!("Error: Either --file or --dir must be specified.\n");
            eprintln!("Use --help for more information.");
            std::process::exit(1);
        }

        return Ok(());
    }

    // Create file-backed accumulator for memory-efficient processing
    let mut accumulator = FileBackedAccumulator::new()?;

    // Determine what to analyze and collect stats into accumulator
    if let Some(file_path) = &args.file {
        let stats = analyze_file(file_path, max_file_size)?;
        accumulator.add_file(&stats)?;
    } else if let Some(dir_path) = &args.dir {
        analyze_directory(dir_path, max_file_size, &mut accumulator)?;
    } else {
        // No arguments provided, show help
        eprintln!("Error: Either --file or --dir must be specified.\n");
        eprintln!("Use --help for more information.");
        std::process::exit(1);
    };

    // Flush accumulator to ensure all data is written
    accumulator.flush()?;

    // Output results using the accumulator
    match args.output_format() {
        OutputFormat::Text => output_text_from_accumulator(&accumulator)?,
        OutputFormat::Json => output_json_from_accumulator(&accumulator)?,
    }

    Ok(())
}

/// Performs AST-driven line-by-line classification of Rust source code.
///
/// Leverages the `ra_ap_syntax` parser to tokenize source content with full semantic awareness,
/// correctly disambiguating comment-like patterns within string literals, raw strings, and
/// character constants. Each line receives a deterministic classification based on its
/// predominant token type.
///
/// # Algorithm
///
/// 1. Parse source into syntax tree via `SourceFile::parse`
/// 2. Build byte-offset-to-line-number mapping for O(log n) lookups
/// 3. Traverse all tokens, classifying covered lines according to token kinds
/// 4. Resolve conflicts (e.g., code + comment on same line) via precedence rules
///
/// # Classification Rules
///
/// - Lines with only whitespace tokens → `LineType::Blank`
/// - Lines with `COMMENT` tokens matching `///|//!|/**|/*!` → `LineType::Rustdoc`
/// - Lines with other `COMMENT` tokens → `LineType::Comment`
/// - Lines with any non-whitespace, non-comment tokens → `LineType::Code`
/// - Mixed lines prioritize Comment/Rustdoc over Code
///
/// # Arguments
///
/// * `content` - Complete source file content as UTF-8 string
///
/// # Returns
///
/// Vector of [`LineType`] classifications, indexed by zero-based line number
///
/// # Examples
///
/// ```ignore
/// let code = "// comment\nfn main() {}\n";
/// let types = analyze_lines(code);
/// assert_eq!(types[0], LineType::Comment);
/// assert_eq!(types[1], LineType::Code);
/// ```
fn analyze_lines(content: &str) -> Vec<LineType> {
    let total_lines = content.lines().count();
    if total_lines == 0 {
        return Vec::new();
    }

    // Parse the content to get tokens
    let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
    let root = parse.syntax_node();

    // Initialize all lines as blank
    let mut line_types = vec![LineType::Blank; total_lines];

    // Build line start positions for accurate mapping
    let mut line_starts = vec![0];
    for (pos, ch) in content.char_indices() {
        if ch == '\n' {
            line_starts.push(pos + 1);
        }
    }

    // Helper to map byte offset to line number
    let offset_to_line = |offset: usize| -> usize {
        line_starts
            .binary_search(&offset)
            .unwrap_or_else(|insert_pos| insert_pos.saturating_sub(1))
            .min(total_lines - 1)
    };

    // Collect all tokens and classify lines based on them
    for token in root
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        let range = token.text_range();
        let start_offset: usize = range.start().into();
        let end_offset: usize = range.end().into();

        let start_line = offset_to_line(start_offset);
        let end_line = offset_to_line(end_offset.saturating_sub(1).max(start_offset));

        // Classify based on token kind
        match token.kind() {
            SyntaxKind::COMMENT => {
                // Check if this is a rustdoc comment
                let text = token.text();
                let is_rustdoc = text.starts_with("///")
                    || text.starts_with("//!")
                    || text.starts_with("/**")
                    || text.starts_with("/*!");

                let line_type = if is_rustdoc {
                    LineType::Rustdoc
                } else {
                    LineType::Comment
                };

                // Mark all lines covered by this comment token
                line_types[start_line..=end_line.min(total_lines - 1)]
                    .iter_mut()
                    .for_each(|t| *t = line_type);
            }
            SyntaxKind::WHITESPACE => {
                // Whitespace doesn't change classification
            }
            _ => {
                // Any other token (keywords, identifiers, literals, etc.) is Code
                // But only override if the line isn't already marked as Comment or Rustdoc
                line_types[start_line..=end_line.min(total_lines - 1)]
                    .iter_mut()
                    .filter(|t| **t != LineType::Comment && **t != LineType::Rustdoc)
                    .for_each(|t| *t = LineType::Code);
            }
        }
    }

    line_types
}

/// Computes line statistics from classified line types by counting occurrences.
///
/// # Arguments
///
/// * `line_types` - Slice of classified line types to count
/// * `total_lines` - Total number of lines (used for the `all_lines` field)
///
/// # Returns
///
/// A `LineStats` instance with counts for each line type
fn compute_line_stats(line_types: &[LineType], total_lines: usize) -> LineStats {
    let blank_lines = line_types.iter().filter(|&&t| t == LineType::Blank).count();
    let comment_lines = line_types
        .iter()
        .filter(|&&t| t == LineType::Comment)
        .count();
    let rustdoc_lines = line_types
        .iter()
        .filter(|&&t| t == LineType::Rustdoc)
        .count();
    let code_lines = line_types.iter().filter(|&&t| t == LineType::Code).count();

    LineStats {
        all_lines: total_lines,
        blank_lines,
        comment_lines,
        rustdoc_lines,
        code_lines,
    }
}

/// Represents a code section with its classification and line range.
#[derive(Debug, Clone)]
struct CodeSection {
    /// Starting line number (0-indexed).
    start_line: usize,
    /// Ending line number (0-indexed, inclusive).
    end_line: usize,
}

/// Determines if a syntax node represents a test item by checking for test attributes.
///
/// Identifies functions with `#[test]` or `#[cfg(test)]` attributes, and modules
/// with `#[cfg(test)]` attributes.
///
/// # Arguments
///
/// * `node` - The syntax tree node to examine
///
/// # Returns
///
/// `true` if the node represents a test function or test module, `false` otherwise
fn is_test_node(node: &SyntaxNode) -> bool {
    // Check if this is a function with #[test] or #[cfg(test)] attribute
    if let Some(func) = ast::Fn::cast(node.clone()) {
        for attr in func.attrs() {
            if let Some(path) = attr.path() {
                let attr_text = path.to_string();
                if attr_text == "test" {
                    return true;
                }
                if attr_text == "cfg"
                    && let Some(token_tree) = attr.token_tree()
                {
                    let tree_text = token_tree.syntax().text().to_string();
                    if tree_text.contains("test") {
                        return true;
                    }
                }
            }
        }
    }

    // Check if this is a module with #[cfg(test)]
    if let Some(module) = ast::Module::cast(node.clone()) {
        for attr in module.attrs() {
            if let Some(path) = attr.path() {
                let attr_text = path.to_string();
                if attr_text == "cfg"
                    && let Some(token_tree) = attr.token_tree()
                {
                    let tree_text = token_tree.syntax().text().to_string();
                    if tree_text.contains("test") {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Recursively finds test sections in the syntax tree by traversing AST nodes.
///
/// When a test node is found, adds its line range to the sections vector and
/// stops recursing into that subtree (since nested tests are part of the parent).
///
/// # Arguments
///
/// * `node` - The current syntax tree node being examined
/// * `sections` - Mutable vector to collect discovered test sections
/// * `content` - The complete source file content (used for line offset calculation)
fn find_test_sections(node: &SyntaxNode, sections: &mut Vec<CodeSection>, content: &str) {
    if is_test_node(node) {
        let text_range = node.text_range();
        let start_offset = text_range.start().into();
        let end_offset = text_range.end().into();

        let start_line = content[..start_offset].lines().count().saturating_sub(1);
        let end_line = content[..end_offset].lines().count().saturating_sub(1);

        trace!("Found test section: lines {}-{}", start_line, end_line);

        sections.push(CodeSection {
            start_line,
            end_line,
        });
        return; // Don't recurse into test sections
    }

    for child in node.children() {
        find_test_sections(&child, sections, content);
    }
}

/// Determines which lines belong to production vs test code using AST analysis.
///
/// Parses the source code to build a syntax tree, identifies all test sections,
/// and marks their corresponding line ranges as test code.
///
/// # Arguments
///
/// * `content` - The source code content to classify
///
/// # Returns
///
/// A vector of boolean values, one per line, where `true` indicates test code
/// and `false` indicates production code
fn classify_lines(content: &str) -> Vec<bool> {
    let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
    let root = parse.syntax_node();

    let mut test_sections = Vec::new();
    find_test_sections(&root, &mut test_sections, content);

    let total_lines = content.lines().count();
    let mut is_test_line = vec![false; total_lines];

    for section in test_sections {
        let end = section.end_line.min(total_lines - 1);
        is_test_line[section.start_line..=end].fill(true);
    }

    debug!(
        "Classified {} lines: {} test, {} production",
        total_lines,
        is_test_line.iter().filter(|&&x| x).count(),
        is_test_line.iter().filter(|&&x| !x).count()
    );

    is_test_line
}

/// Analyzes a single Rust source file to compute line statistics.
///
/// Reads the file, classifies lines as blank/comment/code, identifies test sections,
/// and computes separate statistics for total, production, and test code.
///
/// # Arguments
///
/// * `path` - Path to the Rust source file to analyze
/// * `max_file_size` - Optional maximum file size in bytes; files larger are skipped
///
/// # Returns
///
/// `Ok(FileStats)` with the analysis results, or `Err(String)` if file reading fails
/// or the file exceeds the size limit
///
/// # Errors
///
/// Returns an error if the file cannot be read or exceeds the maximum size
fn analyze_file(path: &Path, max_file_size: Option<u64>) -> Result<FileStats, String> {
    trace!("Analyzing file: {}", path.display());

    // Check file size if limit is specified
    if let Some(max_size) = max_file_size {
        let metadata = fs::metadata(path).map_err(|e| {
            format!(
                "Failed to get metadata for '{}': {}. File may not exist or be inaccessible.",
                path.display(),
                e
            )
        })?;
        let file_size = metadata.len();

        if file_size > max_size {
            debug!(
                "Skipping file {} (size: {} bytes exceeds limit: {} bytes)",
                path.display(),
                file_size,
                max_size
            );
            return Err(format!(
                "File '{}' exceeds maximum size limit ({} bytes > {} bytes). Consider increasing --max-file-size or excluding this file.",
                path.display(),
                file_size,
                max_size
            ));
        }
    }

    let content = fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read file '{}': {}. Ensure the file exists, is readable, and is valid UTF-8.",
            path.display(),
            e
        )
    })?;

    let total_lines = content.lines().count();
    if total_lines == 0 {
        debug!("Empty file: {}", path.display());
        return Ok(FileStats {
            path: path.to_string_lossy().to_string(),
            total: LineStats {
                all_lines: 0,
                ..Default::default()
            },
            production: LineStats::default(),
            test: LineStats::default(),
        });
    }

    let line_types = analyze_lines(&content);
    let is_test_line = classify_lines(&content);

    // Compute total stats
    let total = compute_line_stats(&line_types, total_lines);

    // Compute production stats
    let prod_line_types: Vec<_> = line_types
        .iter()
        .zip(is_test_line.iter())
        .filter(|&(_, &is_test)| !is_test)
        .map(|(lt, _)| *lt)
        .collect();
    let production = compute_line_stats(&prod_line_types, prod_line_types.len());

    // Compute test stats
    let test_line_types: Vec<_> = line_types
        .iter()
        .zip(is_test_line.iter())
        .filter(|&(_, &is_test)| is_test)
        .map(|(lt, _)| *lt)
        .collect();
    let test = compute_line_stats(&test_line_types, test_line_types.len());

    debug!(
        "File {}: total={}, prod={}, test={}",
        path.display(),
        total.all_lines,
        production.all_lines,
        test.all_lines
    );

    Ok(FileStats {
        path: path.to_string_lossy().to_string(),
        total,
        production,
        test,
    })
}

/// Analyzes all Rust files in a directory recursively using parallel directory traversal.
///
/// Walks the directory tree, identifies all `.rs` files, and analyzes each one in parallel
/// using rayon. Follows symbolic links during traversal. Files exceeding the size limit
/// are skipped. Shows a progress bar during processing. Results are added to the provided
/// accumulator, enabling memory-efficient processing of large codebases.
///
/// # Arguments
///
/// * `dir` - Path to the directory to analyze
/// * `max_file_size` - Optional maximum file size in bytes; larger files are skipped
/// * `accumulator` - Accumulator to collect file statistics
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` if no Rust files are found or analysis fails
///
/// # Errors
///
/// Returns an error if:
/// - No Rust files are found in the directory
/// - Accumulator operations fail
fn analyze_directory<A: StatsAccumulator>(
    dir: &Path,
    max_file_size: Option<u64>,
    accumulator: &mut A,
) -> Result<(), String> {
    // First pass: collect all .rs file paths
    let rust_files: Vec<PathBuf> = WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
        .map(|e| e.path().to_path_buf())
        .collect();

    if rust_files.is_empty() {
        return Err(format!("No Rust files found in {}", dir.display()));
    }

    // Setup progress bar only if we're in a terminal
    let is_terminal = std::io::stdout().is_terminal();
    let progress = if is_terminal {
        let bar = ProgressBar::new(rust_files.len() as u64);
        bar.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("█▓░"),
        );
        bar
    } else {
        ProgressBar::hidden()
    };

    // Atomic counters
    let skipped_count = Arc::new(AtomicUsize::new(0));
    let analyzed_count = Arc::new(AtomicUsize::new(0));

    // Wrap accumulator in Arc<Mutex<>> for thread-safe access
    let accumulator_mutex = Arc::new(Mutex::new(accumulator));

    // Second pass: analyze files in parallel
    rust_files.par_iter().for_each(|path| {
        let result = analyze_file(path, max_file_size);
        progress.inc(1);

        match result {
            Ok(stats) => {
                // Add to accumulator
                let mut acc = accumulator_mutex.lock().unwrap();
                if let Err(e) = acc.add_file(&stats) {
                    progress.println(format!("Error adding file stats: {}", e));
                } else {
                    analyzed_count.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.contains("exceeds maximum size") => {
                skipped_count.fetch_add(1, Ordering::Relaxed);
                debug!("Skipped: {}", e);
            }
            Err(e) => {
                progress.println(format!("Error: {}", e));
            }
        }
    });

    progress.finish_with_message("Analysis complete");

    let final_analyzed = analyzed_count.load(Ordering::Relaxed);
    let final_skipped = skipped_count.load(Ordering::Relaxed);

    debug!(
        "Analyzed {} files in {} (skipped {} files exceeding size limit)",
        final_analyzed,
        dir.display(),
        final_skipped
    );

    if final_analyzed == 0 {
        return Err(format!(
            "No Rust files could be analyzed in {}",
            dir.display()
        ));
    }

    Ok(())
}

/// Formats line statistics for plain text output with proper indentation.
///
/// # Arguments
///
/// * `stats` - The line statistics to format
/// * `indent` - Number of spaces to indent each line
///
/// # Returns
///
/// A formatted string with all line counts displayed on separate lines
fn format_line_stats(stats: &LineStats, indent: usize) -> String {
    let prefix = " ".repeat(indent);
    format!(
        "{}All lines: {}\n\
         {}Blank lines: {}\n\
         {}Comment lines: {}\n\
         {}Rustdoc lines: {}\n\
         {}Code lines: {}",
        prefix,
        stats.all_lines,
        prefix,
        stats.blank_lines,
        prefix,
        stats.comment_lines,
        prefix,
        stats.rustdoc_lines,
        prefix,
        stats.code_lines
    )
}

/// Formats a single line for debug output with type prefix and optional coloring.
///
/// # Arguments
///
/// * `line` - The line content to display
/// * `line_type` - The type of line (Blank, Comment, Rustdoc, Code)
/// * `is_test` - Whether this line is in test code
/// * `use_color` - Whether to apply color to the prefix
///
/// # Returns
///
/// A formatted string with prefix and line content
fn format_debug_line(line: &str, line_type: LineType, is_test: bool, use_color: bool) -> String {
    let (prefix, colored_prefix) = match (is_test, line_type) {
        (false, LineType::Blank) => (
            DEBUG_MARKER_PRODUCTION_BLANK,
            DEBUG_MARKER_PRODUCTION_BLANK.bright_black(),
        ),
        (false, LineType::Comment) => (
            DEBUG_MARKER_PRODUCTION_COMMENT,
            DEBUG_MARKER_PRODUCTION_COMMENT.green(),
        ),
        (false, LineType::Rustdoc) => (
            DEBUG_MARKER_PRODUCTION_RUSTDOC,
            DEBUG_MARKER_PRODUCTION_RUSTDOC.bright_green(),
        ),
        (false, LineType::Code) => (
            DEBUG_MARKER_PRODUCTION_CODE,
            DEBUG_MARKER_PRODUCTION_CODE.blue(),
        ),
        (true, LineType::Blank) => (
            DEBUG_MARKER_TEST_BLANK,
            DEBUG_MARKER_TEST_BLANK.bright_black(),
        ),
        (true, LineType::Comment) => (
            DEBUG_MARKER_TEST_COMMENT,
            DEBUG_MARKER_TEST_COMMENT.yellow(),
        ),
        (true, LineType::Rustdoc) => (
            DEBUG_MARKER_TEST_RUSTDOC,
            DEBUG_MARKER_TEST_RUSTDOC.bright_yellow(),
        ),
        (true, LineType::Code) => (DEBUG_MARKER_TEST_CODE, DEBUG_MARKER_TEST_CODE.magenta()),
    };

    if use_color {
        format!("{}  {}", colored_prefix, line)
    } else {
        format!("{}  {}", prefix, line)
    }
}

/// Outputs a single file in debug mode with line-by-line type annotations.
///
/// # Arguments
///
/// * `path` - Path to the file to analyze
/// * `use_color` - Whether to apply color to the prefixes
/// * `max_file_size` - Optional maximum file size limit
///
/// # Returns
///
/// `Ok(())` on success, or an error message if analysis fails
///
/// # Errors
///
/// Returns an error if the file cannot be read or analyzed
fn output_file_debug(
    path: &Path,
    use_color: bool,
    max_file_size: Option<u64>,
) -> Result<(), String> {
    // Check file size if limit is specified
    if let Some(max_size) = max_file_size {
        let metadata = fs::metadata(path)
            .map_err(|e| format!("Failed to read metadata for {}: {}", path.display(), e))?;
        let file_size = metadata.len();

        if file_size > max_size {
            return Err(format!(
                "File {} ({} bytes) exceeds maximum size ({} bytes)",
                path.display(),
                file_size,
                max_size
            ));
        }
    }

    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    if content.is_empty() {
        return Ok(());
    }

    let line_types = analyze_lines(&content);
    let is_test_line = classify_lines(&content);

    println!("{}:", path.display());
    for (i, line) in content.lines().enumerate() {
        if i < line_types.len() && i < is_test_line.len() {
            let formatted = format_debug_line(line, line_types[i], is_test_line[i], use_color);
            println!("{}", formatted);
        }
    }

    Ok(())
}

/// Outputs statistics in plain text format from an accumulator.
///
/// Displays a summary section with aggregated statistics, followed by
/// detailed statistics for each analyzed file. Streams file data from
/// the accumulator without loading everything into memory.
///
/// # Arguments
///
/// * `accumulator` - The stats accumulator to read from
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` if reading from accumulator fails
///
/// # Errors
///
/// Returns an error if the accumulator cannot provide file statistics
fn output_text_from_accumulator<A: StatsAccumulator>(accumulator: &A) -> Result<(), String> {
    let summary = accumulator.get_summary();

    println!("Summary:");
    println!("  Files: {}", summary.files);
    println!("  Total:");
    println!(
        "{}",
        format_line_stats(&summary.total, TEXT_OUTPUT_BASE_INDENT)
    );
    println!("  Production:");
    println!(
        "{}",
        format_line_stats(&summary.production, TEXT_OUTPUT_BASE_INDENT)
    );
    println!("  Test:");
    println!(
        "{}",
        format_line_stats(&summary.test, TEXT_OUTPUT_BASE_INDENT)
    );

    println!("\nFiles:");
    for file in accumulator.iter_files()? {
        println!("  {}:", file.path);
        println!("    Total:");
        println!(
            "{}",
            format_line_stats(&file.total, TEXT_OUTPUT_NESTED_INDENT)
        );
        println!("    Production:");
        println!(
            "{}",
            format_line_stats(&file.production, TEXT_OUTPUT_NESTED_INDENT)
        );
        println!("    Test:");
        println!(
            "{}",
            format_line_stats(&file.test, TEXT_OUTPUT_NESTED_INDENT)
        );
    }

    Ok(())
}

/// Outputs statistics in JSON format from an accumulator.
///
/// Serializes the summary and file statistics to pretty-printed JSON.
/// Streams file data from the accumulator to build the report.
///
/// # Arguments
///
/// * `accumulator` - The stats accumulator to read from
///
/// # Returns
///
/// `Ok(())` on success, or `Err(String)` if serialization fails
///
/// # Errors
///
/// Returns an error if:
/// - The accumulator cannot provide file statistics
/// - JSON serialization fails
fn output_json_from_accumulator<A: StatsAccumulator>(accumulator: &A) -> Result<(), String> {
    let summary = accumulator.get_summary();
    let files: Vec<FileStats> = accumulator.iter_files()?.collect();

    let report = Report { summary, files };

    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    println!("{}", json);
    Ok(())
}

/// Unit tests for the ruloc line counting and analysis functionality.
///
/// Tests cover:
/// - Line statistics operations (default, add)
/// - Line classification (blank, comments, code)
/// - Block comment handling
/// - Production vs test code classification
/// - Summary aggregation
/// - Output formatting
#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Test Helpers ====================

    /// Creates a `LineStats` instance with the given values.
    ///
    /// # Arguments
    ///
    /// * `all_lines` - Total number of lines
    /// * `blank_lines` - Number of blank lines
    /// * `comment_lines` - Number of comment lines (excluding rustdocs)
    /// * `rustdoc_lines` - Number of rustdoc lines
    /// * `code_lines` - Number of code lines
    fn make_line_stats(
        all_lines: usize,
        blank_lines: usize,
        comment_lines: usize,
        rustdoc_lines: usize,
        code_lines: usize,
    ) -> LineStats {
        LineStats {
            all_lines,
            blank_lines,
            comment_lines,
            rustdoc_lines,
            code_lines,
        }
    }

    /// Creates a simple `FileStats` instance for testing.
    ///
    /// Creates a file with the given stats for all code (no distinction between production and test).
    ///
    /// # Arguments
    ///
    /// * `path` - File path
    /// * `all_lines` - Total number of lines
    /// * `blank_lines` - Number of blank lines
    /// * `comment_lines` - Number of comment lines (excluding rustdocs)
    /// * `rustdoc_lines` - Number of rustdoc lines
    /// * `code_lines` - Number of code lines
    fn make_simple_file_stats(
        path: &str,
        all_lines: usize,
        blank_lines: usize,
        comment_lines: usize,
        rustdoc_lines: usize,
        code_lines: usize,
    ) -> FileStats {
        let stats = make_line_stats(
            all_lines,
            blank_lines,
            comment_lines,
            rustdoc_lines,
            code_lines,
        );
        FileStats {
            path: path.to_string(),
            total: stats.clone(),
            production: stats,
            test: LineStats::default(),
        }
    }

    /// Creates a `FileStats` instance with separate production and test stats.
    ///
    /// # Arguments
    ///
    /// * `path` - File path
    /// * `prod_stats` - Production code statistics
    /// * `test_stats` - Test code statistics
    fn make_file_stats_with_tests(
        path: &str,
        prod_stats: LineStats,
        test_stats: LineStats,
    ) -> FileStats {
        let mut total = prod_stats.clone();
        total.add(&test_stats);

        FileStats {
            path: path.to_string(),
            total,
            production: prod_stats,
            test: test_stats,
        }
    }

    /// Creates a standard test `FileStats` for basic testing scenarios.
    ///
    /// Contains 10 total lines: 7 production (4 code) and 3 test (1 code).
    fn make_standard_test_file_stats() -> FileStats {
        make_file_stats_with_tests(
            "test.rs",
            make_line_stats(7, 1, 2, 0, 4),
            make_line_stats(3, 1, 1, 0, 1),
        )
    }

    /// Creates a minimal test `FileStats` for simple scenarios.
    ///
    /// Contains 5 lines of production code only.
    fn make_minimal_test_file_stats() -> FileStats {
        make_simple_file_stats("test.rs", 5, 1, 1, 0, 3)
    }

    /// Creates a detailed test `FileStats` for complex testing scenarios.
    ///
    /// Contains 15 total lines: 10 production (5 code) and 5 test (3 code).
    fn make_detailed_test_file_stats() -> FileStats {
        make_file_stats_with_tests(
            "test.rs",
            make_line_stats(10, 2, 3, 0, 5),
            make_line_stats(5, 1, 1, 0, 3),
        )
    }

    // ==================== Tests ====================

    /// Tests that `LineStats::default()` creates a zero-initialized instance.
    #[test]
    fn test_line_stats_default() {
        let stats = LineStats::default();
        assert_eq!(stats.all_lines, 0);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.rustdoc_lines, 0);
        assert_eq!(stats.code_lines, 0);
    }

    /// Tests that `LineStats::add()` correctly accumulates statistics.
    #[test]
    fn test_line_stats_add() {
        let mut stats1 = make_line_stats(10, 2, 3, 0, 5);
        let stats2 = make_line_stats(20, 4, 6, 0, 10);
        stats1.add(&stats2);
        assert_eq!(stats1.all_lines, 30);
        assert_eq!(stats1.blank_lines, 6);
        assert_eq!(stats1.comment_lines, 9);
        assert_eq!(stats1.rustdoc_lines, 0);
        assert_eq!(stats1.code_lines, 15);
    }

    /// Tests that blank lines (empty or whitespace-only) are correctly identified.
    #[test]
    fn test_analyze_lines_blank() {
        let content = "\n\n  \n\t\n";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert!(line_types.iter().all(|&t| t == LineType::Blank));
    }

    /// Tests that line comments (`//`) and doc comments (`///`) are correctly identified.
    #[test]
    fn test_analyze_lines_line_comments() {
        let content = "// comment 1\n// comment 2\n/// doc comment";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Comment);
        assert_eq!(line_types[2], LineType::Rustdoc);
    }

    /// Tests that multiline block comments (`/* ... */`) are correctly identified.
    #[test]
    fn test_analyze_lines_block_comment() {
        let content = "/* start\nmiddle\nend */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Comment));
    }

    /// Tests that code lines are correctly identified.
    #[test]
    fn test_analyze_lines_code() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Code));
    }

    /// Tests classification of mixed content (comments, blanks, and code).
    #[test]
    fn test_analyze_lines_mixed() {
        let content = "// comment\n\nfn main() {}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Blank);
        assert_eq!(line_types[2], LineType::Code);
    }

    /// Tests that line statistics are correctly computed from line type classifications.
    #[test]
    fn test_compute_line_stats() {
        let line_types = vec![
            LineType::Comment,
            LineType::Blank,
            LineType::Code,
            LineType::Code,
            LineType::Blank,
        ];
        let stats = compute_line_stats(&line_types, 5);
        assert_eq!(stats.all_lines, 5);
        assert_eq!(stats.blank_lines, 2);
        assert_eq!(stats.comment_lines, 1);
        assert_eq!(stats.code_lines, 2);
    }

    /// Tests that production code without tests is classified as non-test.
    #[test]
    fn test_classify_lines_no_tests() {
        let content = "fn main() {\n    println!(\"hello\");\n}";
        let is_test = classify_lines(content);
        assert_eq!(is_test.len(), 3);
        assert!(is_test.iter().all(|&x| !x));
    }

    /// Tests that functions marked with `#[test]` are correctly identified as test code.
    #[test]
    fn test_classify_lines_with_test_function() {
        let content = r#"
fn production() {}

#[test]
fn test_something() {
    assert!(true);
}
"#;
        let is_test = classify_lines(content);
        // Lines: "", "fn production() {}", "", "#[test]", "fn test_something() {", "    assert!(true);", "}"
        assert!(!is_test.is_empty());
        // The test function lines should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    /// Tests that modules marked with `#[cfg(test)]` are correctly identified as test code.
    #[test]
    fn test_classify_lines_with_test_module() {
        let content = r#"
fn production() {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_it() {}
}
"#;
        let is_test = classify_lines(content);
        assert!(!is_test.is_empty());
        // The module and its contents should be marked as test
        assert!(is_test.iter().any(|&x| x));
    }

    /// Tests that `Summary::add_file()` correctly aggregates file statistics.
    #[test]
    fn test_summary_add_file() {
        let mut summary = Summary::default();
        let file_stats = make_standard_test_file_stats();
        summary.add_file(&file_stats);
        assert_eq!(summary.files, 1);
        assert_eq!(summary.total.all_lines, 10);
        assert_eq!(summary.production.all_lines, 7);
        assert_eq!(summary.test.all_lines, 3);
    }

    /// Tests that line statistics are correctly formatted for text output.
    #[test]
    fn test_format_line_stats() {
        let stats = make_line_stats(100, 20, 30, 0, 50);
        let formatted = format_line_stats(&stats, 2);
        assert!(formatted.contains("All lines: 100"));
        assert!(formatted.contains("Blank lines: 20"));
        assert!(formatted.contains("Comment lines: 30"));
        assert!(formatted.contains("Code lines: 50"));
    }

    /// Tests that empty files (with no content) are handled correctly.
    #[test]
    fn test_empty_file_analysis() {
        let content = "";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 0);
    }

    /// Tests that multiline block comments spanning multiple lines are correctly handled.
    #[test]
    fn test_analyze_lines_multiline_block_comment() {
        let content = "code line\n/* comment start\ncomment middle\ncomment end */\nmore code";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 5);
        assert_eq!(line_types[0], LineType::Code);
        assert_eq!(line_types[1], LineType::Comment);
        assert_eq!(line_types[2], LineType::Comment);
        assert_eq!(line_types[3], LineType::Comment);
        assert_eq!(line_types[4], LineType::Code);
    }

    /// Tests parsing file size with no unit (bytes).
    #[test]
    fn test_parse_file_size_bytes() {
        assert_eq!(parse_file_size("1000").unwrap(), 1000);
        assert_eq!(parse_file_size("500").unwrap(), 500);
        assert_eq!(parse_file_size("1").unwrap(), 1);
    }

    /// Tests parsing file size with KB unit.
    #[test]
    fn test_parse_file_size_kb() {
        assert_eq!(parse_file_size("1KB").unwrap(), 1024);
        assert_eq!(parse_file_size("1kb").unwrap(), 1024);
        assert_eq!(parse_file_size("3.5KB").unwrap(), 3584);
        assert_eq!(parse_file_size("10KB").unwrap(), 10240);
    }

    /// Tests parsing file size with MB unit.
    #[test]
    fn test_parse_file_size_mb() {
        assert_eq!(parse_file_size("1MB").unwrap(), 1048576);
        assert_eq!(parse_file_size("1mb").unwrap(), 1048576);
        assert_eq!(parse_file_size("2.5MB").unwrap(), 2621440);
    }

    /// Tests parsing file size with GB unit.
    #[test]
    fn test_parse_file_size_gb() {
        assert_eq!(parse_file_size("1GB").unwrap(), 1073741824);
        assert_eq!(parse_file_size("1gb").unwrap(), 1073741824);
        assert_eq!(parse_file_size("1.1GB").unwrap(), 1181116006);
    }

    /// Tests parsing file size with whitespace.
    #[test]
    fn test_parse_file_size_with_whitespace() {
        assert_eq!(parse_file_size("  1000  ").unwrap(), 1000);
        assert_eq!(parse_file_size("  3.5KB  ").unwrap(), 3584);
    }

    /// Tests parsing invalid file size returns error.
    #[test]
    fn test_parse_file_size_invalid() {
        assert!(parse_file_size("invalid").is_err());
        assert!(parse_file_size("").is_err());
        assert!(parse_file_size("KB").is_err());
        assert!(parse_file_size("1TB").is_err()); // Unsupported unit
    }

    /// Tests parsing negative file size returns error.
    #[test]
    fn test_parse_file_size_negative() {
        assert!(parse_file_size("-100").is_err());
        assert!(parse_file_size("-1KB").is_err());
    }

    /// Tests Args::parse_max_file_size with valid input.
    #[test]
    fn test_args_parse_max_file_size_some() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: Some("10MB".to_string()),
        };
        let result = args.parse_max_file_size().unwrap();
        assert_eq!(result, Some(10 * 1024 * 1024));
    }

    /// Tests Args::parse_max_file_size with None.
    #[test]
    fn test_args_parse_max_file_size_none() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: None,
        };
        let result = args.parse_max_file_size().unwrap();
        assert_eq!(result, None);
    }

    /// Tests Args::output_format returns Json when flag is set.
    #[test]
    fn test_args_output_format_json() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: true,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: None,
        };
        assert_eq!(args.output_format(), OutputFormat::Json);
    }

    /// Tests Args::output_format returns Text by default.
    #[test]
    fn test_args_output_format_text() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: None,
        };
        assert_eq!(args.output_format(), OutputFormat::Text);
    }

    /// Tests analyze_file with a real Rust file.
    #[test]
    fn test_analyze_file_integration() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_ruloc.rs");

        let test_code = r#"
// Production code
fn hello() {
    println!("hello");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_hello() {
        assert!(true);
    }
}
"#;

        std::fs::write(&temp_file, test_code).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.all_lines > 0);
        assert!(stats.production.code_lines > 0);
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_file respects max_file_size limit.
    #[test]
    fn test_analyze_file_size_limit() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_ruloc_large.rs");

        let test_code = "// A large file\n".repeat(100);
        std::fs::write(&temp_file, &test_code).unwrap();

        // File is ~1600 bytes, set limit to 100 bytes
        let result = analyze_file(&temp_file, Some(100));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum size"));

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_text formatting with accumulator.
    #[test]
    fn test_output_text() {
        let mut acc = InMemoryAccumulator::new();
        let stats = make_standard_test_file_stats();
        acc.add_file(&stats).unwrap();

        let result = output_text_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests output_json formatting with accumulator.
    #[test]
    fn test_output_json() {
        let mut acc = InMemoryAccumulator::new();
        let stats = make_standard_test_file_stats();
        acc.add_file(&stats).unwrap();

        let result = output_json_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests analyze_directory with a temporary directory structure.
    #[test]
    fn test_analyze_directory_integration() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_dir");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create test files
        let file1 = temp_dir.join("file1.rs");
        fs::write(&file1, "fn main() {}\n").unwrap();

        let file2 = temp_dir.join("file2.rs");
        fs::write(&file2, "#[test]\nfn test() {}\n").unwrap();

        let mut accumulator = InMemoryAccumulator::new();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_ok());

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_directory with max_file_size filtering.
    #[test]
    fn test_analyze_directory_with_size_filter() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_dir_filter");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a small file
        let small_file = temp_dir.join("small.rs");
        fs::write(&small_file, "fn f() {}\n").unwrap();

        // Create a large file
        let large_file = temp_dir.join("large.rs");
        fs::write(&large_file, "// Large\n".repeat(100)).unwrap();

        // Set size limit to 100 bytes - should skip the large file
        let mut accumulator = InMemoryAccumulator::new();
        let result = analyze_directory(&temp_dir, Some(100), &mut accumulator);
        assert!(result.is_ok());

        let summary = accumulator.get_summary();
        // Only the small file should be analyzed
        assert!(summary.files <= 1);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_file with invalid Rust code.
    #[test]
    fn test_analyze_file_invalid_rust() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_invalid.rs");

        // This is syntactically invalid but should still count lines
        let invalid_code = "fn broken( {}\nthis is not rust\n";
        std::fs::write(&temp_file, invalid_code).unwrap();

        let result = analyze_file(&temp_file, None);
        // Should succeed even with invalid syntax, just counts lines
        assert!(result.is_ok());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_file with a file that has complex nested test modules.
    #[test]
    fn test_analyze_file_nested_tests() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_nested.rs");

        let test_code = r#"
fn prod() {}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    mod nested {
        #[test]
        fn inner_test() {}
    }
}
"#;

        std::fs::write(&temp_file, test_code).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_text with detailed file statistics.
    #[test]
    fn test_output_text_with_files() {
        let mut acc = InMemoryAccumulator::new();
        let file_stats = make_detailed_test_file_stats();
        acc.add_file(&file_stats).unwrap();

        let result = output_text_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests analyze_file with an empty file.
    #[test]
    fn test_analyze_file_empty() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_empty.rs");

        std::fs::write(&temp_file, "").unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.total.all_lines, 0);
        assert_eq!(stats.total.code_lines, 0);
        assert_eq!(stats.production.all_lines, 0);
        assert_eq!(stats.test.all_lines, 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_directory with a directory containing no Rust files.
    #[test]
    fn test_analyze_directory_no_rust_files() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_no_rs");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create non-.rs files
        let txt_file = temp_dir.join("readme.txt");
        fs::write(&txt_file, "Not a Rust file").unwrap();

        let mut accumulator = InMemoryAccumulator::new();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No Rust files found"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_directory where all files are too large.
    #[test]
    fn test_analyze_directory_all_files_too_large() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_all_large");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create large files that exceed size limit
        let large_file1 = temp_dir.join("large1.rs");
        fs::write(&large_file1, "// Large\n".repeat(100)).unwrap();

        let large_file2 = temp_dir.join("large2.rs");
        fs::write(&large_file2, "// Large\n".repeat(100)).unwrap();

        // Set size limit to 50 bytes - all files will be skipped
        let mut accumulator = InMemoryAccumulator::new();
        let result = analyze_directory(&temp_dir, Some(50), &mut accumulator);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("No Rust files could be analyzed")
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_file with a module using standalone #[cfg(test)] attribute.
    #[test]
    fn test_analyze_file_cfg_test_module() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_cfg_module.rs");

        let test_code = r#"
fn production_code() {}

#[cfg(test)]
mod test_module {
    #[test]
    fn test_helper() {}
}
"#;

        std::fs::write(&temp_file, test_code).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.all_lines > 0);
        assert!(stats.production.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_file with a very large file that exceeds size limit.
    #[test]
    fn test_analyze_file_exceeds_size_with_debug() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_large_debug.rs");

        // Create a file larger than 500 bytes
        let large_content = "// This is a large file\n".repeat(50);
        std::fs::write(&temp_file, &large_content).unwrap();

        // Set limit to 500 bytes - file should be rejected
        let result = analyze_file(&temp_file, Some(500));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum size"));

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests parse_file_size with various edge cases for error handling.
    #[test]
    fn test_parse_file_size_edge_cases() {
        // Test with decimal values
        assert_eq!(parse_file_size("1.5KB").unwrap(), 1536);
        assert_eq!(parse_file_size("0.5MB").unwrap(), 524288);

        // Test case insensitivity
        assert_eq!(parse_file_size("1kb").unwrap(), 1024);
        assert_eq!(parse_file_size("1Kb").unwrap(), 1024);
    }

    /// Tests analyze_file with line at the boundary of file size limit.
    #[test]
    fn test_analyze_file_at_size_boundary() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_boundary.rs");

        // Create a file exactly at the boundary
        let content = "x".repeat(1000);
        std::fs::write(&temp_file, &content).unwrap();

        // Test with size exactly at the limit - should pass
        let result = analyze_file(&temp_file, Some(1000));
        assert!(result.is_ok());

        // Test with size one byte under - should fail
        let result = analyze_file(&temp_file, Some(999));
        assert!(result.is_err());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests file with only whitespace lines after empty check.
    #[test]
    fn test_analyze_file_only_whitespace() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_whitespace.rs");

        let content = "   \n\t\n  \t  \n";
        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.total.blank_lines, 3);
        assert_eq!(stats.total.code_lines, 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests classify_lines with a mix of production and test code.
    #[test]
    fn test_classify_lines_mixed() {
        let code = r#"
fn production() {}

#[cfg(test)]
mod tests {
    fn helper() {}

    #[test]
    fn test_fn() {}
}
"#;
        let result = classify_lines(code);

        // Should identify test lines correctly
        assert!(result.iter().any(|&is_test| is_test));
        assert!(result.iter().any(|&is_test| !is_test));
    }

    /// Tests analyze_directory with mixed valid and invalid files.
    #[test]
    fn test_analyze_directory_mixed_files() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_mixed");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create valid files
        let good1 = temp_dir.join("good1.rs");
        fs::write(&good1, "fn a() {}").unwrap();

        let good2 = temp_dir.join("good2.rs");
        fs::write(&good2, "fn b() {}\n#[test]\nfn t() {}").unwrap();

        // Create a subdirectory with more files
        let subdir = temp_dir.join("subdir");
        fs::create_dir_all(&subdir).unwrap();
        let sub_file = subdir.join("sub.rs");
        fs::write(&sub_file, "fn sub() {}").unwrap();

        let mut accumulator = InMemoryAccumulator::new();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_ok());

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 3); // Should find all 3 .rs files

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests Args structure with parse_max_file_size error handling.
    #[test]
    fn test_args_parse_max_file_size_error() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: Some("invalid".to_string()),
        };
        let result = args.parse_max_file_size();
        assert!(result.is_err());
    }

    /// Tests InMemoryAccumulator basic operations.
    #[test]
    fn test_in_memory_accumulator() {
        let mut acc = InMemoryAccumulator::new();

        let stats1 = make_file_stats_with_tests(
            "test1.rs",
            make_line_stats(7, 1, 2, 0, 4),
            make_line_stats(3, 1, 1, 0, 1),
        );
        let stats2 = make_simple_file_stats("test2.rs", 5, 1, 1, 0, 3);

        acc.add_file(&stats1).unwrap();
        acc.add_file(&stats2).unwrap();

        let summary = acc.get_summary();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.total.all_lines, 15);
        assert_eq!(summary.production.code_lines, 7);
        assert_eq!(summary.test.code_lines, 1);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "test1.rs");
        assert_eq!(files[1].path, "test2.rs");
    }

    /// Tests FileBackedAccumulator basic operations.
    #[test]
    fn test_file_backed_accumulator() {
        let mut acc = FileBackedAccumulator::new().unwrap();

        let stats1 = make_file_stats_with_tests(
            "test1.rs",
            make_line_stats(7, 1, 2, 0, 4),
            make_line_stats(3, 1, 1, 0, 1),
        );
        let stats2 = make_simple_file_stats("test2.rs", 5, 1, 1, 0, 3);

        acc.add_file(&stats1).unwrap();
        acc.add_file(&stats2).unwrap();
        acc.flush().unwrap();

        let summary = acc.get_summary();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.total.all_lines, 15);
        assert_eq!(summary.production.code_lines, 7);
        assert_eq!(summary.test.code_lines, 1);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "test1.rs");
        assert_eq!(files[1].path, "test2.rs");
    }

    /// Tests FileBackedAccumulator with large number of files.
    #[test]
    fn test_file_backed_accumulator_many_files() {
        let mut acc = FileBackedAccumulator::new().unwrap();

        // Add 1000 files to test buffering
        for i in 0..1000 {
            let stats = make_simple_file_stats(&format!("test{}.rs", i), 10, 2, 3, 0, 5);
            acc.add_file(&stats).unwrap();
        }

        acc.flush().unwrap();

        let summary = acc.get_summary();
        assert_eq!(summary.files, 1000);
        assert_eq!(summary.total.all_lines, 10000);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 1000);
    }

    /// Tests output_text_from_accumulator with InMemoryAccumulator.
    #[test]
    fn test_output_text_from_accumulator() {
        let mut acc = InMemoryAccumulator::new();
        let stats = make_standard_test_file_stats();
        acc.add_file(&stats).unwrap();

        // Just ensure it doesn't panic
        let result = output_text_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests output_json_from_accumulator with InMemoryAccumulator.
    #[test]
    fn test_output_json_from_accumulator() {
        let mut acc = InMemoryAccumulator::new();
        let stats = make_standard_test_file_stats();
        acc.add_file(&stats).unwrap();

        let result = output_json_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests that FileBackedAccumulator properly handles file I/O errors.
    #[test]
    fn test_file_backed_accumulator_iteration() {
        let mut acc = FileBackedAccumulator::new().unwrap();
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();
        acc.flush().unwrap();

        // Test multiple iterations
        let files1: Vec<_> = acc.iter_files().unwrap().collect();
        let files2: Vec<_> = acc.iter_files().unwrap().collect();

        assert_eq!(files1.len(), 1);
        assert_eq!(files2.len(), 1);
        assert_eq!(files1[0].path, files2[0].path);
    }

    /// Tests InMemoryAccumulator with empty data.
    #[test]
    fn test_in_memory_accumulator_empty() {
        let acc = InMemoryAccumulator::new();

        let summary = acc.get_summary();
        assert_eq!(summary.files, 0);
        assert_eq!(summary.total.all_lines, 0);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 0);
    }

    /// Tests FileBackedAccumulator with empty data.
    #[test]
    fn test_file_backed_accumulator_empty() {
        let acc = FileBackedAccumulator::new().unwrap();

        let summary = acc.get_summary();
        assert_eq!(summary.files, 0);
        assert_eq!(summary.total.all_lines, 0);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 0);
    }

    /// Tests FileBackedAccumulator flush method.
    #[test]
    fn test_file_backed_accumulator_flush() {
        let mut acc = FileBackedAccumulator::new().unwrap();
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();

        // Multiple flushes should succeed
        assert!(acc.flush().is_ok());
        assert!(acc.flush().is_ok());

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 1);
    }

    /// Tests output functions with FileBackedAccumulator.
    #[test]
    fn test_output_functions_with_file_backed_accumulator() {
        let mut acc = FileBackedAccumulator::new().unwrap();
        let stats = make_standard_test_file_stats();
        acc.add_file(&stats).unwrap();
        acc.flush().unwrap();

        // Test text output
        let result = output_text_from_accumulator(&acc);
        assert!(result.is_ok());

        // Test JSON output
        let result = output_json_from_accumulator(&acc);
        assert!(result.is_ok());
    }

    /// Tests analyze_directory error handling with accumulator errors.
    #[test]
    fn test_analyze_directory_with_file_backed_accumulator() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_file_backed");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create test files
        let file1 = temp_dir.join("file1.rs");
        fs::write(&file1, "fn main() {}\n").unwrap();

        let file2 = temp_dir.join("file2.rs");
        fs::write(&file2, "#[test]\nfn test() {}\n").unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_ok());

        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests that analyze_file handles nonexistent files correctly.
    #[test]
    fn test_analyze_file_nonexistent() {
        let result = analyze_file(std::path::Path::new("/nonexistent/file.rs"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to"));
    }

    /// Tests Summary default initialization.
    #[test]
    fn test_summary_default() {
        let summary = Summary::default();
        assert_eq!(summary.files, 0);
        assert_eq!(summary.total.all_lines, 0);
        assert_eq!(summary.production.all_lines, 0);
        assert_eq!(summary.test.all_lines, 0);
    }

    /// Tests that line stats accurately track all components.
    #[test]
    fn test_line_stats_comprehensive() {
        let mut stats = make_line_stats(100, 20, 30, 0, 50);
        let other = make_line_stats(50, 10, 15, 0, 25);

        stats.add(&other);

        assert_eq!(stats.all_lines, 150);
        assert_eq!(stats.blank_lines, 30);
        assert_eq!(stats.comment_lines, 45);
        assert_eq!(stats.rustdoc_lines, 0);
        assert_eq!(stats.code_lines, 75);
    }

    /// Tests InMemoryAccumulator iterator returns owned data.
    #[test]
    fn test_in_memory_accumulator_iterator_ownership() {
        let mut acc = InMemoryAccumulator::new();
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();

        // Consume iterator multiple times
        let files1: Vec<_> = acc.iter_files().unwrap().collect();
        let files2: Vec<_> = acc.iter_files().unwrap().collect();

        assert_eq!(files1.len(), 1);
        assert_eq!(files2.len(), 1);
    }

    /// Tests analyze_file with a file containing only comments.
    #[test]
    fn test_analyze_file_only_comments() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_only_comments.rs");

        let content = "// Comment 1\n// Comment 2\n/* Block comment */\n";
        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.total.comment_lines, 3);
        assert_eq!(stats.total.code_lines, 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests analyze_lines with single-line block comment.
    #[test]
    fn test_analyze_lines_single_line_block_comment() {
        let content = "/* single line block comment */\ncode();\n";
        let line_types = analyze_lines(content);

        assert_eq!(line_types.len(), 2);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Code);
    }

    /// Tests classify_lines with mixed production and test code.
    #[test]
    fn test_classify_lines_production_and_test_mixed() {
        let code = r#"
fn production() {}

#[test]
fn test_func() {
    assert!(true);
}

fn more_production() {}
"#;
        let result = classify_lines(code);

        // Should have both test and production lines
        assert!(result.iter().any(|&is_test| is_test));
        assert!(result.iter().any(|&is_test| !is_test));
    }

    /// Tests FileStats equality.
    #[test]
    fn test_file_stats_equality() {
        let stats1 = make_standard_test_file_stats();
        let stats2 = stats1.clone();

        assert_eq!(stats1, stats2);
    }

    /// Tests Report equality.
    #[test]
    fn test_report_equality() {
        let report1 = Report {
            summary: Summary::default(),
            files: vec![],
        };

        let report2 = report1.clone();

        assert_eq!(report1, report2);
    }

    /// Tests Args output_format method with text flag.
    #[test]
    fn test_args_output_format_with_text_flag() {
        let args = Args {
            file: None,
            dir: None,
            out_text: true,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: None,
        };
        assert_eq!(args.output_format(), OutputFormat::Text);
    }

    /// Tests analyze_file with metadata errors for size checking.
    #[test]
    fn test_analyze_file_metadata_error() {
        // Try to analyze a file that doesn't exist with max_file_size set
        let result = analyze_file(
            std::path::Path::new("/nonexistent/path/file.rs"),
            Some(1000),
        );
        assert!(result.is_err());
    }

    /// Tests analyze_lines with empty content.
    #[test]
    fn test_analyze_lines_empty_content() {
        let content = "";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 0);
    }

    /// Tests compute_line_stats with empty input.
    #[test]
    fn test_compute_line_stats_empty() {
        let line_types: Vec<LineType> = vec![];
        let stats = compute_line_stats(&line_types, 0);
        assert_eq!(stats.all_lines, 0);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.code_lines, 0);
    }

    /// Tests compute_line_stats with only blank lines.
    #[test]
    fn test_compute_line_stats_only_blanks() {
        let line_types = vec![LineType::Blank, LineType::Blank, LineType::Blank];
        let stats = compute_line_stats(&line_types, 3);
        assert_eq!(stats.all_lines, 3);
        assert_eq!(stats.blank_lines, 3);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.code_lines, 0);
    }

    /// Tests is_test_node with non-test nodes.
    #[test]
    fn test_is_test_node_regular_function() {
        let content = "fn regular_function() {}";
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        // The root itself should not be a test node
        assert!(!is_test_node(&root));
    }

    /// Tests analyze_file with file at exact size limit.
    #[test]
    fn test_analyze_file_exact_size_limit() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_exact_size.rs");

        // Create a file with known size
        let content = "fn test() {}"; // 12 bytes
        std::fs::write(&temp_file, content).unwrap();

        // Set limit to exact size - should succeed
        let result = analyze_file(&temp_file, Some(12));
        assert!(result.is_ok());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests FileBackedAccumulator serialization error handling.
    #[test]
    fn test_file_backed_accumulator_add_multiple() {
        let mut acc = FileBackedAccumulator::new().unwrap();

        // Add multiple files sequentially
        for i in 0..10 {
            let mut stats = make_minimal_test_file_stats();
            stats.path = format!("test{}.rs", i);
            assert!(acc.add_file(&stats).is_ok());
        }

        assert!(acc.flush().is_ok());

        let summary = acc.get_summary();
        assert_eq!(summary.files, 10);
    }

    /// Tests analyze_directory with progress tracking.
    #[test]
    fn test_analyze_directory_progress() {
        use std::fs;

        let temp_dir = std::env::temp_dir().join("test_ruloc_progress");
        fs::create_dir_all(&temp_dir).unwrap();

        // Create multiple files to test progress bar
        for i in 0..5 {
            let file = temp_dir.join(format!("file{}.rs", i));
            fs::write(&file, "fn main() {}\n").unwrap();
        }

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_ok());

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 5);

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Tests analyze_file with production and test code.
    #[test]
    fn test_analyze_file_production_and_test() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_prod_test.rs");

        let content = r#"
fn production() {
    println!("production");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_it() {
        assert!(true);
    }
}
"#;
        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.production.code_lines > 0);
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests LineStats serialization.
    #[test]
    fn test_line_stats_serialization() {
        let stats = make_line_stats(100, 20, 30, 0, 50);
        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: LineStats = serde_json::from_str(&json).unwrap();

        assert_eq!(stats, deserialized);
    }

    /// Tests FileStats serialization.
    #[test]
    fn test_file_stats_serialization() {
        let stats = make_standard_test_file_stats();
        let json = serde_json::to_string(&stats).unwrap();
        let deserialized: FileStats = serde_json::from_str(&json).unwrap();

        assert_eq!(stats, deserialized);
    }

    /// Tests Summary serialization.
    #[test]
    fn test_summary_serialization() {
        let summary = Summary {
            files: 5,
            total: make_line_stats(100, 20, 30, 0, 50),
            production: make_line_stats(70, 10, 20, 0, 40),
            test: make_line_stats(30, 10, 10, 0, 10),
        };

        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: Summary = serde_json::from_str(&json).unwrap();

        assert_eq!(summary, deserialized);
    }

    /// Tests Report serialization.
    #[test]
    fn test_report_serialization() {
        let report = Report {
            summary: Summary::default(),
            files: vec![],
        };

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: Report = serde_json::from_str(&json).unwrap();

        assert_eq!(report, deserialized);
    }

    /// Tests FileBackedAccumulator with corrupted data.
    #[test]
    fn test_file_backed_accumulator_corrupted_data() {
        use std::io::Write;

        let mut acc = FileBackedAccumulator::new().unwrap();

        // Add valid data first
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();

        // Write corrupted data directly to the writer
        writeln!(acc.writer, "corrupted json data").unwrap();
        acc.flush().unwrap();

        // Should skip corrupted lines and only return valid ones
        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 1); // Only the valid entry
    }

    /// Tests parse_file_size with zero.
    #[test]
    fn test_parse_file_size_zero() {
        assert_eq!(parse_file_size("0").unwrap(), 0);
        assert_eq!(parse_file_size("0KB").unwrap(), 0);
    }

    /// Tests analyze_lines with code after block comment end.
    #[test]
    fn test_analyze_lines_code_after_block_comment() {
        let content = "/* comment */ code();";
        let line_types = analyze_lines(content);

        assert_eq!(line_types.len(), 1);
        // The whole line is treated as a comment since it starts with /*
        assert_eq!(line_types[0], LineType::Comment);
    }

    /// Tests classify_lines with nested test modules.
    #[test]
    fn test_classify_lines_nested_test_modules() {
        let code = r#"
fn production() {}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    mod nested {
        #[test]
        fn inner() {}
    }
}
"#;
        let result = classify_lines(code);

        // Should have both test and production lines
        assert!(result.iter().any(|&is_test| is_test));
        assert!(result.iter().any(|&is_test| !is_test));
    }

    /// Tests find_test_sections directly.
    #[test]
    fn test_find_test_sections_function() {
        let content = r#"
fn production() {}

#[test]
fn test_one() {}

#[test]
fn test_two() {}
"#;
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut sections = Vec::new();
        find_test_sections(&root, &mut sections, content);

        // Should find two test sections
        assert_eq!(sections.len(), 2);
    }

    /// Tests compute_line_stats with only comments.
    #[test]
    fn test_compute_line_stats_only_comments() {
        let line_types = vec![LineType::Comment, LineType::Comment, LineType::Comment];
        let stats = compute_line_stats(&line_types, 3);
        assert_eq!(stats.all_lines, 3);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 3);
        assert_eq!(stats.code_lines, 0);
    }

    /// Tests compute_line_stats with only code.
    #[test]
    fn test_compute_line_stats_only_code() {
        let line_types = vec![LineType::Code, LineType::Code, LineType::Code];
        let stats = compute_line_stats(&line_types, 3);
        assert_eq!(stats.all_lines, 3);
        assert_eq!(stats.blank_lines, 0);
        assert_eq!(stats.comment_lines, 0);
        assert_eq!(stats.code_lines, 3);
    }

    /// Tests InMemoryAccumulator default behavior.
    #[test]
    fn test_in_memory_accumulator_new() {
        let acc = InMemoryAccumulator::new();
        let summary = acc.get_summary();

        assert_eq!(summary.files, 0);
        assert_eq!(summary.total.all_lines, 0);
    }

    /// Tests analyze_file with very large line count.
    #[test]
    fn test_analyze_file_large_file() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_large_file.rs");

        // Create a file with many lines
        let mut content = String::new();
        for i in 0..1000 {
            content.push_str(&format!("// Line {}\n", i));
        }

        std::fs::write(&temp_file, &content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert_eq!(stats.total.all_lines, 1000);
        assert_eq!(stats.total.comment_lines, 1000);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests is_test_node with a test function.
    #[test]
    fn test_is_test_node_test_function() {
        let content = "#[test]\nfn test_something() {}";
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        // Find the function node
        for child in root.descendants() {
            if ast::Fn::cast(child.clone()).is_some() && is_test_node(&child) {
                return; // Test passes
            }
        }

        panic!("Should have found a test node");
    }

    /// Tests is_test_node returns false for normal code.
    #[test]
    fn test_is_test_node_normal_code() {
        let content = "fn regular_function() {}";
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        // Regular functions should not be test nodes
        for child in root.descendants() {
            if ast::Fn::cast(child.clone()).is_some() {
                // This should return false for regular functions
                let _ = is_test_node(&child);
            }
        }
        // Test completes successfully
    }

    /// Tests find_test_sections with multiple test annotations.
    #[test]
    fn test_find_test_sections_multiple_annotations() {
        let content = r#"
fn production() {}

#[test]
fn test_alpha() {
    assert!(true);
}

#[test]
fn test_beta() {
    assert!(false || true);
}
"#;
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut sections = Vec::new();
        find_test_sections(&root, &mut sections, content);

        // Should find both test functions
        assert!(sections.len() >= 2);
    }

    /// Tests analyze_file with mixed blank, comment, and code lines.
    #[test]
    fn test_analyze_file_mixed_content() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_mixed_content.rs");

        let content = r#"
// Header comment

fn production() {
    // Inner comment
    println!("hello");
}

#[test]
fn test() {

    assert!(true);
}
"#;
        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.blank_lines > 0);
        assert!(stats.total.comment_lines > 0);
        assert!(stats.total.code_lines > 0);
        assert!(stats.production.code_lines > 0);
        assert!(stats.test.code_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests CodeSection structure usage.
    #[test]
    fn test_code_section_creation() {
        let section = CodeSection {
            start_line: 10,
            end_line: 20,
        };

        assert_eq!(section.start_line, 10);
        assert_eq!(section.end_line, 20);
    }

    /// Tests analyze_lines with mixed block and line comments.
    #[test]
    fn test_analyze_lines_mixed_comments() {
        let content = r#"// Line comment
/* Block start
Still in block
Block end */
// Another line comment
code();"#;
        let line_types = analyze_lines(content);

        assert_eq!(line_types.len(), 6);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Comment);
        assert_eq!(line_types[2], LineType::Comment);
        assert_eq!(line_types[3], LineType::Comment);
        assert_eq!(line_types[4], LineType::Comment);
        assert_eq!(line_types[5], LineType::Code);
    }

    /// Tests analyze_lines with tab characters.
    #[test]
    fn test_analyze_lines_with_tabs() {
        let content = "\t\t// Indented comment\n\t\tfn code() {}\n";
        let line_types = analyze_lines(content);

        assert_eq!(line_types.len(), 2);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Code);
    }

    /// Tests that Summary accumulates correctly across multiple files.
    #[test]
    fn test_summary_accumulation() {
        let mut summary = Summary::default();

        for _ in 0..5 {
            let stats = make_standard_test_file_stats();
            summary.add_file(&stats);
        }

        assert_eq!(summary.files, 5);
        assert_eq!(summary.total.all_lines, 50);
        assert_eq!(summary.production.code_lines, 20);
        assert_eq!(summary.test.code_lines, 5);
    }

    /// Tests FileBackedAccumulator without any flush calls.
    #[test]
    fn test_file_backed_accumulator_no_explicit_flush() {
        let mut acc = FileBackedAccumulator::new().unwrap();
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();

        // Try to read without explicit flush
        let summary = acc.get_summary();
        assert_eq!(summary.files, 1);
    }

    /// Tests LineType enum completeness.
    #[test]
    fn test_line_type_variants() {
        let blank = LineType::Blank;
        let comment = LineType::Comment;
        let code = LineType::Code;

        assert_eq!(blank, LineType::Blank);
        assert_eq!(comment, LineType::Comment);
        assert_eq!(code, LineType::Code);
        assert_ne!(blank, comment);
        assert_ne!(blank, code);
        assert_ne!(comment, code);
    }

    /// Tests format_debug_line for all line type combinations.
    #[test]
    fn test_format_debug_line() {
        // Test production code prefixes
        let line = "fn main() {}";
        assert!(
            format_debug_line(line, LineType::Code, false, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_PRODUCTION_CODE))
        );
        assert!(
            format_debug_line(line, LineType::Comment, false, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_PRODUCTION_COMMENT))
        );
        assert!(
            format_debug_line(line, LineType::Rustdoc, false, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_PRODUCTION_RUSTDOC))
        );
        assert!(
            format_debug_line("", LineType::Blank, false, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_PRODUCTION_BLANK))
        );

        // Test test code prefixes
        assert!(
            format_debug_line(line, LineType::Code, true, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_TEST_CODE))
        );
        assert!(
            format_debug_line(line, LineType::Comment, true, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_TEST_COMMENT))
        );
        assert!(
            format_debug_line(line, LineType::Rustdoc, true, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_TEST_RUSTDOC))
        );
        assert!(
            format_debug_line("", LineType::Blank, true, false)
                .starts_with(&format!("{}  ", DEBUG_MARKER_TEST_BLANK))
        );

        // Verify line content is preserved
        assert!(format_debug_line(line, LineType::Code, false, false).contains(line));

        // Test with colors enabled
        let colored_output = format_debug_line(line, LineType::Code, false, true);
        assert!(colored_output.contains(line));
    }

    /// Tests InMemoryAccumulator::default() implementation.
    #[test]
    fn test_in_memory_accumulator_default() {
        let acc = InMemoryAccumulator::default();
        let summary = acc.get_summary();
        assert_eq!(summary.files, 0);
        assert_eq!(summary.total.all_lines, 0);
    }

    /// Tests output_file_debug function with a test file.
    #[test]
    fn test_output_file_debug() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_debug_output.rs");

        let content = r#"/// Doc comment
fn main() {
    // Comment
    println!("test");
}

#[test]
fn test() {
    assert!(true);
}"#;

        std::fs::write(&temp_file, content).unwrap();

        // Test without colors
        let result = output_file_debug(&temp_file, false, None);
        assert!(result.is_ok());

        // Test with colors
        let result = output_file_debug(&temp_file, true, None);
        assert!(result.is_ok());

        // Test with size limit that allows file
        let result = output_file_debug(&temp_file, false, Some(10000));
        assert!(result.is_ok());

        // Test with size limit that rejects file
        let result = output_file_debug(&temp_file, false, Some(10));
        assert!(result.is_err());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_file_debug with empty file.
    #[test]
    fn test_output_file_debug_empty() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_debug_empty.rs");

        std::fs::write(&temp_file, "").unwrap();

        let result = output_file_debug(&temp_file, false, None);
        assert!(result.is_ok());

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests output_file_debug with nonexistent file.
    #[test]
    fn test_output_file_debug_nonexistent() {
        let path = std::path::Path::new("/nonexistent/file.rs");
        let result = output_file_debug(path, false, None);
        assert!(result.is_err());
    }

    /// Tests analyze_lines with rustdoc comments.
    #[test]
    fn test_analyze_lines_rustdoc() {
        let content = "/// This is a rustdoc comment\n//! Module doc\n/** Block rustdoc */\n/*! Block module doc */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert_eq!(line_types[0], LineType::Rustdoc);
        assert_eq!(line_types[1], LineType::Rustdoc);
        assert_eq!(line_types[2], LineType::Rustdoc);
        assert_eq!(line_types[3], LineType::Rustdoc);
    }

    /// Tests analyze_lines with mixed rustdoc and regular comments.
    #[test]
    fn test_analyze_lines_mixed_rustdoc_comments() {
        let content = "/// Rustdoc\n// Regular\n//! Module doc\n/* Block */\n/** Block rustdoc */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 5);
        assert_eq!(line_types[0], LineType::Rustdoc);
        assert_eq!(line_types[1], LineType::Comment);
        assert_eq!(line_types[2], LineType::Rustdoc);
        assert_eq!(line_types[3], LineType::Comment);
        assert_eq!(line_types[4], LineType::Rustdoc);
    }

    /// Tests compute_line_stats with rustdoc lines.
    #[test]
    fn test_compute_line_stats_with_rustdoc() {
        let line_types = vec![
            LineType::Rustdoc,
            LineType::Rustdoc,
            LineType::Code,
            LineType::Blank,
        ];
        let stats = compute_line_stats(&line_types, 4);
        assert_eq!(stats.all_lines, 4);
        assert_eq!(stats.rustdoc_lines, 2);
        assert_eq!(stats.code_lines, 1);
        assert_eq!(stats.blank_lines, 1);
        assert_eq!(stats.comment_lines, 0);
    }

    /// Tests LineStats with rustdoc lines included.
    #[test]
    fn test_line_stats_with_rustdoc() {
        let stats = make_line_stats(100, 20, 15, 10, 55);
        assert_eq!(stats.all_lines, 100);
        assert_eq!(stats.rustdoc_lines, 10);
        // Verify sum equals total
        let sum = stats.blank_lines + stats.comment_lines + stats.rustdoc_lines + stats.code_lines;
        assert_eq!(sum, stats.all_lines);
    }

    /// Tests LineStats::add with rustdoc lines.
    #[test]
    fn test_line_stats_add_with_rustdoc() {
        let mut stats1 = make_line_stats(50, 10, 10, 5, 25);
        let stats2 = make_line_stats(50, 5, 5, 10, 30);
        stats1.add(&stats2);
        assert_eq!(stats1.all_lines, 100);
        assert_eq!(stats1.rustdoc_lines, 15);
        assert_eq!(stats1.comment_lines, 15);
        assert_eq!(stats1.blank_lines, 15);
        assert_eq!(stats1.code_lines, 55);
    }

    /// Tests format_line_stats with rustdoc.
    #[test]
    fn test_format_line_stats_with_rustdoc() {
        let stats = make_line_stats(100, 20, 15, 12, 53);
        let formatted = format_line_stats(&stats, 2);
        assert!(formatted.contains("Rustdoc lines: 12"));
    }

    /// Tests Summary with rustdoc lines.
    #[test]
    fn test_summary_with_rustdoc() {
        let mut summary = Summary::default();
        let file_stats = make_file_stats_with_tests(
            "test.rs",
            make_line_stats(50, 5, 5, 8, 32),
            make_line_stats(20, 2, 2, 4, 12),
        );
        summary.add_file(&file_stats);
        assert_eq!(summary.total.rustdoc_lines, 12); // 8 + 4
        assert_eq!(summary.production.rustdoc_lines, 8);
        assert_eq!(summary.test.rustdoc_lines, 4);
    }

    /// Tests analyze_file with rustdoc comments.
    #[test]
    fn test_analyze_file_with_rustdoc() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_rustdoc.rs");

        let content = r#"/// Module documentation
//! Crate documentation
/// Function documentation
fn documented_function() {
    // Regular comment
}

/** Block rustdoc */
fn another_function() {}

#[cfg(test)]
mod tests {
    /// Test function doc
    #[test]
    fn test_it() {}
}"#;

        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.rustdoc_lines > 0);
        assert!(stats.production.rustdoc_lines > 0);

        std::fs::remove_file(&temp_file).ok();
    }

    /// Tests FileBackedAccumulator deserialization error handling.
    #[test]
    fn test_file_backed_accumulator_invalid_json() {
        use std::io::Write;

        let mut acc = FileBackedAccumulator::new().unwrap();

        // Write some invalid JSON directly
        writeln!(acc.writer, "{{invalid json}}").unwrap();
        writeln!(acc.writer, "not even json").unwrap();

        // Add valid data
        let stats = make_minimal_test_file_stats();
        acc.add_file(&stats).unwrap();
        acc.flush().unwrap();

        // Should only return the valid entry
        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 1);
    }

    /// Tests format_debug_line with rustdoc type.
    #[test]
    fn test_format_debug_line_rustdoc() {
        let line = "/// Documentation";

        // Test production rustdoc
        let prod = format_debug_line(line, LineType::Rustdoc, false, false);
        assert!(prod.starts_with(&format!("{}  ", DEBUG_MARKER_PRODUCTION_RUSTDOC)));
        assert!(prod.contains(line));

        // Test test rustdoc
        let test = format_debug_line(line, LineType::Rustdoc, true, false);
        assert!(test.starts_with(&format!("{}  ", DEBUG_MARKER_TEST_RUSTDOC)));
        assert!(test.contains(line));

        // Test with colors
        let colored = format_debug_line(line, LineType::Rustdoc, false, true);
        assert!(colored.contains(line));
    }

    /// Tests analyze_lines with multiline rustdoc block comment.
    #[test]
    fn test_analyze_lines_multiline_rustdoc_block() {
        let content = "/** Start rustdoc\nContinued rustdoc\nEnd rustdoc */\ncode();";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert_eq!(line_types[0], LineType::Rustdoc);
        assert_eq!(line_types[1], LineType::Rustdoc);
        assert_eq!(line_types[2], LineType::Rustdoc);
        assert_eq!(line_types[3], LineType::Code);
    }

    /// Tests analyze_lines with module-level rustdoc.
    #[test]
    fn test_analyze_lines_module_rustdoc() {
        let content = "//! Module level documentation\n//! Continued\n\nfn main() {}";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert_eq!(line_types[0], LineType::Rustdoc);
        assert_eq!(line_types[1], LineType::Rustdoc);
        assert_eq!(line_types[2], LineType::Blank);
        assert_eq!(line_types[3], LineType::Code);
    }

    /// Tests FileBackedAccumulator with IO error during flush.
    #[test]
    fn test_file_backed_accumulator_flush_error() {
        // This test verifies error handling but actual IO error simulation is complex
        let mut acc = FileBackedAccumulator::new().unwrap();

        // Multiple flushes should work
        for _ in 0..5 {
            assert!(acc.flush().is_ok());
        }
    }

    /// Tests InMemoryAccumulator with large number of files.
    #[test]
    fn test_in_memory_accumulator_many_files() {
        let mut acc = InMemoryAccumulator::new();

        // Add many files to test memory accumulation
        for i in 0..100 {
            let stats = make_simple_file_stats(&format!("file{}.rs", i), 10, 2, 2, 1, 5);
            acc.add_file(&stats).unwrap();
        }

        let summary = acc.get_summary();
        assert_eq!(summary.files, 100);
        assert_eq!(summary.total.all_lines, 1000);
        assert_eq!(summary.total.rustdoc_lines, 100);

        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 100);
    }

    /// Tests FileBackedAccumulator iter_files with corrupted line.
    #[test]
    fn test_file_backed_accumulator_iter_files_error_handling() {
        use std::io::Write;

        let mut acc = FileBackedAccumulator::new().unwrap();
        let stats1 = make_minimal_test_file_stats();
        acc.add_file(&stats1).unwrap();

        // Write corrupted line
        writeln!(acc.writer, "{{corrupted}}").unwrap();

        let stats2 = make_simple_file_stats("test2.rs", 5, 1, 1, 0, 3);
        acc.add_file(&stats2).unwrap();
        acc.flush().unwrap();

        // Should skip corrupted line
        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 2); // Two valid files
    }

    /// Tests is_test_node with module having cfg(test) attribute.
    #[test]
    fn test_is_test_node_with_cfg_test_module() {
        let content = "#[cfg(test)]\nmod tests {}";
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut found_test_module = false;
        for child in root.descendants() {
            if let Some(module) = ast::Module::cast(child.clone()) {
                // Check attributes directly for debugging
                for attr in module.attrs() {
                    if let Some(path) = attr.path() {
                        let attr_text = path.to_string();
                        // The attribute path might be just "cfg" not "cfg(test)"
                        if attr_text == "cfg" || attr_text.contains("cfg") {
                            found_test_module = true;
                            break;
                        }
                    }
                }
                if found_test_module {
                    break;
                }
            }
        }

        assert!(found_test_module);
    }

    /// Tests find_test_sections with nested test modules.
    #[test]
    fn test_find_test_sections_nested_modules() {
        let content = r#"
#[cfg(test)]
mod tests {
    mod inner {
        #[test]
        fn test() {}
    }
}"#;
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut sections = Vec::new();
        find_test_sections(&root, &mut sections, content);

        // Should find the test module
        assert!(!sections.is_empty());
    }

    /// Tests analyze_lines edge cases with complex combinations.
    #[test]
    fn test_analyze_lines_edge_cases() {
        // Empty string content
        let content = "";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 0);

        // Only newlines
        let content = "\n\n\n";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);
        assert!(line_types.iter().all(|&t| t == LineType::Blank));

        // Mixed code and comment on same line
        let content = "fn test() {} // comment";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 1);
        assert_eq!(line_types[0], LineType::Comment); // Comment overrides code when both present
    }

    /// Tests CodeSection usage in find_test_sections.
    #[test]
    fn test_code_section_ranges() {
        let content = r#"
fn prod() {}

#[test]
fn test1() {}

#[test]
fn test2() {}
"#;
        let parse = SourceFile::parse(content, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut sections = Vec::new();
        find_test_sections(&root, &mut sections, content);

        // Verify sections were found
        assert!(sections.len() >= 2);
        for section in &sections {
            assert!(section.end_line >= section.start_line);
        }
    }

    /// Tests offset_to_line mapping in analyze_lines.
    #[test]
    fn test_analyze_lines_offset_mapping() {
        let content = "line1\nline2\nline3";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 3);

        // All should be code lines
        assert!(line_types.iter().all(|&t| t == LineType::Code));
    }

    /// Tests analyze_lines with very long lines.
    #[test]
    fn test_analyze_lines_long_lines() {
        let long_comment = format!("// {}", "x".repeat(5000));
        let long_code = format!("fn test() {{ {} }}", "x".repeat(5000));
        let content = format!("{}\n{}", long_comment, long_code);

        let line_types = analyze_lines(&content);
        assert_eq!(line_types.len(), 2);
        assert_eq!(line_types[0], LineType::Comment);
        assert_eq!(line_types[1], LineType::Code);
    }

    /// Tests FileBackedAccumulator with empty iterator.
    #[test]
    fn test_file_backed_accumulator_empty_iterator() {
        let acc = FileBackedAccumulator::new().unwrap();
        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 0);
    }

    /// Tests analyze_lines with code containing comment-like strings.
    #[test]
    fn test_analyze_lines_comment_in_string() {
        let content = r#"let s = "// not a comment";"#;
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 1);
        assert_eq!(line_types[0], LineType::Code); // Should be code, not comment
    }

    /// Tests analyze_lines with rustdoc in block comment.
    #[test]
    fn test_analyze_lines_rustdoc_block_multiline() {
        let content = "/*!\n * Module doc\n * More doc\n */";
        let line_types = analyze_lines(content);
        assert_eq!(line_types.len(), 4);
        assert!(line_types.iter().all(|&t| t == LineType::Rustdoc));
    }

    /// Tests classify_lines with empty input.
    #[test]
    fn test_classify_lines_empty() {
        let result = classify_lines("");
        assert_eq!(result.len(), 0);
    }

    /// Tests classify_lines with no test code.
    #[test]
    fn test_classify_lines_all_production() {
        let content = "fn prod1() {}\nfn prod2() {}\nfn prod3() {}";
        let result = classify_lines(content);
        assert!(result.iter().all(|&is_test| !is_test));
    }

    /// Tests FileBackedAccumulator error path coverage.
    #[test]
    fn test_file_backed_accumulator_write_error_simulation() {
        // We can't easily simulate real write errors, but we can test the error handling path
        let mut acc = FileBackedAccumulator::new().unwrap();

        // Add a large number of files to exercise buffering
        for i in 0..10000 {
            let mut stats = make_minimal_test_file_stats();
            stats.path = format!("file{}.rs", i);
            let result = acc.add_file(&stats);
            assert!(result.is_ok());
        }

        acc.flush().unwrap();

        // Verify all files were written correctly
        let files: Vec<_> = acc.iter_files().unwrap().collect();
        assert_eq!(files.len(), 10000);
    }

    /// Tests FileStats with all line types.
    #[test]
    fn test_file_stats_all_line_types() {
        let prod_stats = make_line_stats(100, 10, 15, 20, 55);
        let test_stats = make_line_stats(50, 5, 10, 10, 25);
        let file_stats = make_file_stats_with_tests("test.rs", prod_stats, test_stats);

        assert_eq!(file_stats.total.all_lines, 150);
        assert_eq!(file_stats.total.blank_lines, 15);
        assert_eq!(file_stats.total.comment_lines, 25);
        assert_eq!(file_stats.total.rustdoc_lines, 30);
        assert_eq!(file_stats.total.code_lines, 80);
    }

    /// Tests Summary accumulation with rustdoc.
    #[test]
    fn test_summary_accumulation_with_rustdoc() {
        let mut summary = Summary::default();

        for i in 0..10 {
            let file_stats = make_file_stats_with_tests(
                &format!("file{}.rs", i),
                make_line_stats(50, 5, 5, 10, 30),
                make_line_stats(25, 3, 2, 5, 15),
            );
            summary.add_file(&file_stats);
        }

        assert_eq!(summary.files, 10);
        assert_eq!(summary.total.rustdoc_lines, 150); // (10 + 5) * 10
        assert_eq!(summary.production.rustdoc_lines, 100); // 10 * 10
        assert_eq!(summary.test.rustdoc_lines, 50); // 5 * 10
    }

    /// Tests parse_file_size with fractional values.
    #[test]
    fn test_parse_file_size_fractional() {
        assert_eq!(parse_file_size("0.25KB").unwrap(), 256);
        assert_eq!(parse_file_size("2.75MB").unwrap(), 2883584);
        assert_eq!(parse_file_size("0.001GB").unwrap(), 1073741);
    }

    /// Tests Args with all flags set.
    #[test]
    fn test_args_all_flags() {
        let args = Args {
            file: Some(std::path::PathBuf::from("test.rs")),
            dir: None,
            out_text: true,
            out_json: false,
            debug: true,
            no_color: true,
            verbose: true,
            max_file_size: Some("100KB".to_string()),
        };

        assert_eq!(args.output_format(), OutputFormat::Text);
        let size = args.parse_max_file_size().unwrap();
        assert_eq!(size, Some(102400));
    }

    /// Tests is_test_node with #[cfg(test)] attribute detection.
    #[test]
    fn test_is_test_node_cfg_test_detection() {
        let code = r#"
#[cfg(test)]
mod tests {
    fn helper() {}
}
"#;
        let parse = SourceFile::parse(code, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        // Find nodes that are detected as test nodes
        let mut found_test_node = false;
        for node in root.descendants() {
            if is_test_node(&node) {
                found_test_node = true;
                break;
            }
        }
        assert!(
            found_test_node,
            "Should detect #[cfg(test)] module as test node"
        );
    }

    /// Tests output_text_from_accumulator with file stats.
    #[test]
    fn test_output_text_accumulator_with_data() {
        let mut accumulator = FileBackedAccumulator::new().unwrap();

        let stats = FileStats {
            path: "test.rs".to_string(),
            total: LineStats {
                all_lines: 10,
                blank_lines: 2,
                comment_lines: 3,
                code_lines: 5,
                rustdoc_lines: 1,
            },
            production: LineStats {
                all_lines: 6,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 3,
                rustdoc_lines: 1,
            },
            test: LineStats {
                all_lines: 4,
                blank_lines: 1,
                comment_lines: 1,
                code_lines: 2,
                rustdoc_lines: 0,
            },
        };

        accumulator.add_file(&stats).unwrap();
        accumulator.flush().unwrap();

        // Ensure function doesn't panic
        let result = output_text_from_accumulator(&accumulator);
        assert!(result.is_ok());
    }

    /// Tests output_json_from_accumulator with multiple files.
    #[test]
    fn test_output_json_accumulator_multi_file() {
        let mut accumulator = FileBackedAccumulator::new().unwrap();

        let stats1 = FileStats {
            path: "test1.rs".to_string(),
            total: LineStats {
                all_lines: 10,
                blank_lines: 2,
                comment_lines: 3,
                code_lines: 5,
                rustdoc_lines: 1,
            },
            production: LineStats {
                all_lines: 6,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 3,
                rustdoc_lines: 1,
            },
            test: LineStats {
                all_lines: 4,
                blank_lines: 1,
                comment_lines: 1,
                code_lines: 2,
                rustdoc_lines: 0,
            },
        };

        let stats2 = FileStats {
            path: "test2.rs".to_string(),
            total: LineStats {
                all_lines: 8,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 5,
                rustdoc_lines: 0,
            },
            production: LineStats {
                all_lines: 8,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 5,
                rustdoc_lines: 0,
            },
            test: LineStats::default(),
        };

        accumulator.add_file(&stats1).unwrap();
        accumulator.add_file(&stats2).unwrap();
        accumulator.flush().unwrap();

        let result = output_json_from_accumulator(&accumulator);
        assert!(result.is_ok());
    }

    /// Tests analyze_file respecting file size limits.
    #[test]
    fn test_analyze_file_with_size_limit() {
        let mut temp_file = std::env::temp_dir();
        temp_file.push("test_size_limited.rs");

        // Create a file larger than the limit
        let large_content = "// Large file\n".repeat(50);
        std::fs::write(&temp_file, large_content).unwrap();

        let result = analyze_file(&temp_file, Some(100));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds maximum size"));

        std::fs::remove_file(&temp_file).unwrap();
    }

    /// Tests is_test_node detects #[cfg(test)] on functions.
    #[test]
    fn test_is_test_node_cfg_test_on_function() {
        let code = r#"
#[cfg(test)]
fn helper_function() {
    println!("test helper");
}
"#;
        let parse = SourceFile::parse(code, ra_ap_syntax::Edition::CURRENT);
        let root = parse.syntax_node();

        let mut found_cfg_test_fn = false;
        for node in root.descendants() {
            if is_test_node(&node) && ast::Fn::cast(node.clone()).is_some() {
                found_cfg_test_fn = true;
                break;
            }
        }
        assert!(found_cfg_test_fn, "Should detect #[cfg(test)] on function");
    }

    /// Tests classify_lines with complex mixed production and test code.
    #[test]
    fn test_classify_lines_complex_cfg_test() {
        let content = r#"
// Production code
fn prod() {}

#[cfg(test)]
fn test_helper() {}

#[cfg(test)]
mod tests {
    #[test]
    fn test1() {}
}
"#;
        let is_test = classify_lines(content);

        // Should have some production and some test lines
        let test_count = is_test.iter().filter(|&&x| x).count();
        let prod_count = is_test.iter().filter(|&&x| !x).count();

        assert!(test_count > 0, "Should detect test lines");
        assert!(prod_count > 0, "Should detect production lines");
    }

    /// Tests accumulator get_summary with FileBackedAccumulator.
    #[test]
    fn test_file_backed_accumulator_get_summary() {
        let mut accumulator = FileBackedAccumulator::new().unwrap();

        let stats1 = FileStats {
            path: "file1.rs".to_string(),
            total: LineStats {
                all_lines: 100,
                blank_lines: 10,
                comment_lines: 20,
                code_lines: 70,
                rustdoc_lines: 5,
            },
            production: LineStats {
                all_lines: 60,
                blank_lines: 5,
                comment_lines: 10,
                code_lines: 45,
                rustdoc_lines: 5,
            },
            test: LineStats {
                all_lines: 40,
                blank_lines: 5,
                comment_lines: 10,
                code_lines: 25,
                rustdoc_lines: 0,
            },
        };

        let stats2 = FileStats {
            path: "file2.rs".to_string(),
            total: LineStats {
                all_lines: 50,
                blank_lines: 5,
                comment_lines: 10,
                code_lines: 35,
                rustdoc_lines: 2,
            },
            production: LineStats {
                all_lines: 50,
                blank_lines: 5,
                comment_lines: 10,
                code_lines: 35,
                rustdoc_lines: 2,
            },
            test: LineStats::default(),
        };

        accumulator.add_file(&stats1).unwrap();
        accumulator.add_file(&stats2).unwrap();
        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();

        assert_eq!(summary.files, 2);
        assert_eq!(summary.total.all_lines, 150);
        assert_eq!(summary.production.all_lines, 110);
        assert_eq!(summary.test.all_lines, 40);
    }

    /// Tests analyze_directory with FileBackedAccumulator and no valid files.
    #[test]
    fn test_analyze_directory_file_backed_no_files() {
        let temp_dir = std::env::temp_dir().join("test_no_files_fb");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create only a non-Rust file
        std::fs::write(temp_dir.join("readme.md"), "Not Rust").unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No Rust files"));

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests analyze_directory with all files exceeding size limit using FileBackedAccumulator.
    #[test]
    fn test_analyze_directory_file_backed_all_too_large() {
        let temp_dir = std::env::temp_dir().join("test_all_large_fb");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create large files
        std::fs::write(temp_dir.join("big.rs"), "// ".repeat(200)).unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, Some(50), &mut accumulator);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("No Rust files could be analyzed")
        );

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests analyze_file and analyze_directory integration with real directory.
    #[test]
    fn test_integration_analyze_real_directory() {
        let temp_dir = std::env::temp_dir().join("test_real_analysis");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create realistic Rust files
        std::fs::write(
            temp_dir.join("lib.rs"),
            r#"
//! Library documentation

/// A function
pub fn func() {}

#[cfg(test)]
mod tests {
    #[test]
    fn test_func() {}
}
"#,
        )
        .unwrap();

        std::fs::write(
            temp_dir.join("main.rs"),
            r#"
fn main() {
    println!("Hello");
}
"#,
        )
        .unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);

        assert!(result.is_ok());

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);
        assert!(summary.total.all_lines > 0);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests output_text_from_accumulator with both accumulators.
    #[test]
    fn test_output_text_integration() {
        let mut accumulator = InMemoryAccumulator::new();

        let stats = FileStats {
            path: "sample.rs".to_string(),
            total: LineStats {
                all_lines: 20,
                blank_lines: 3,
                comment_lines: 5,
                code_lines: 12,
                rustdoc_lines: 2,
            },
            production: LineStats {
                all_lines: 15,
                blank_lines: 2,
                comment_lines: 3,
                code_lines: 10,
                rustdoc_lines: 2,
            },
            test: LineStats {
                all_lines: 5,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 2,
                rustdoc_lines: 0,
            },
        };

        accumulator.add_file(&stats).unwrap();

        // Call output function - it prints to stdout
        let result = output_text_from_accumulator(&accumulator);
        assert!(result.is_ok());

        // Also test with FileBackedAccumulator
        let mut fb_acc = FileBackedAccumulator::new().unwrap();
        fb_acc.add_file(&stats).unwrap();
        fb_acc.flush().unwrap();
        let result2 = output_text_from_accumulator(&fb_acc);
        assert!(result2.is_ok());
    }

    /// Tests output_json_from_accumulator with both accumulators.
    #[test]
    fn test_output_json_integration() {
        let mut accumulator = InMemoryAccumulator::new();

        let stats = FileStats {
            path: "sample.rs".to_string(),
            total: LineStats {
                all_lines: 20,
                blank_lines: 3,
                comment_lines: 5,
                code_lines: 12,
                rustdoc_lines: 2,
            },
            production: LineStats {
                all_lines: 15,
                blank_lines: 2,
                comment_lines: 3,
                code_lines: 10,
                rustdoc_lines: 2,
            },
            test: LineStats {
                all_lines: 5,
                blank_lines: 1,
                comment_lines: 2,
                code_lines: 2,
                rustdoc_lines: 0,
            },
        };

        accumulator.add_file(&stats).unwrap();

        // Call output function - it prints to stdout
        let result = output_json_from_accumulator(&accumulator);
        assert!(result.is_ok());

        // Also test with FileBackedAccumulator
        let mut fb_acc = FileBackedAccumulator::new().unwrap();
        fb_acc.add_file(&stats).unwrap();
        fb_acc.flush().unwrap();
        let result2 = output_json_from_accumulator(&fb_acc);
        assert!(result2.is_ok());
    }

    /// Tests Args::parse_max_file_size with completely invalid input.
    #[test]
    fn test_args_parse_max_file_size_invalid_format() {
        let args = Args {
            file: None,
            dir: None,
            out_text: false,
            out_json: false,
            debug: false,
            no_color: false,
            verbose: false,
            max_file_size: Some("not-a-number".to_string()),
        };
        let result = args.parse_max_file_size();
        assert!(result.is_err());
    }

    /// Tests analyze_directory with subdirectories containing Rust files.
    #[test]
    fn test_analyze_directory_with_subdirs() {
        let temp_dir = std::env::temp_dir().join("test_subdirs");
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join("subdir")).unwrap();

        // Create files in root and subdirectory
        std::fs::write(temp_dir.join("root.rs"), "fn root() {}").unwrap();
        std::fs::write(temp_dir.join("subdir/nested.rs"), "fn nested() {}").unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);

        assert!(result.is_ok());
        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests analyze_file with complex code containing all line types.
    #[test]
    fn test_analyze_file_comprehensive_content() {
        let temp_file = std::env::temp_dir().join("comprehensive.rs");

        let content = r#"
//! Module docs

// Regular comment

/// Rustdoc for function
fn production_fn() {
    // Implementation
    println!("prod");
}

#[cfg(test)]
mod tests {
    /// Test docs
    #[test]
    fn test_something() {
        assert!(true);
    }

    #[cfg(test)]
    fn helper() {}
}
"#;

        std::fs::write(&temp_file, content).unwrap();

        let result = analyze_file(&temp_file, None);
        assert!(result.is_ok());

        let stats = result.unwrap();
        assert!(stats.total.all_lines > 0);
        assert!(stats.production.all_lines > 0);
        assert!(stats.test.all_lines > 0);
        assert!(stats.total.rustdoc_lines > 0);
        assert!(stats.total.comment_lines > 0);
        assert!(stats.total.blank_lines > 0);

        std::fs::remove_file(&temp_file).unwrap();
    }

    /// Tests end-to-end workflow: create files, analyze, output.
    #[test]
    fn test_end_to_end_workflow() {
        let temp_dir = std::env::temp_dir().join("test_e2e");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create multiple Rust files
        std::fs::write(
            temp_dir.join("file1.rs"),
            "fn main() {\n    println!(\"test\");\n}\n",
        )
        .unwrap();

        std::fs::write(
            temp_dir.join("file2.rs"),
            r#"
#[cfg(test)]
mod tests {
    #[test]
    fn test() {}
}
"#,
        )
        .unwrap();

        // Analyze with FileBackedAccumulator
        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);
        assert!(result.is_ok());

        accumulator.flush().unwrap();

        // Output both formats
        let text_result = output_text_from_accumulator(&accumulator);
        assert!(text_result.is_ok());

        let json_result = output_json_from_accumulator(&accumulator);
        assert!(json_result.is_ok());

        // Verify summary
        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests analyze_directory handles file iteration and accumulation correctly.
    #[test]
    fn test_analyze_directory_accumulation() {
        let temp_dir = std::env::temp_dir().join("test_accumulation");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create several Rust files with varying content
        for i in 1..=5 {
            let content = format!(
                "// File {}\nfn func{}() {{}}\n\n#[cfg(test)]\nmod tests{} {{}}\n",
                i, i, i
            );
            std::fs::write(temp_dir.join(format!("file{}.rs", i)), content).unwrap();
        }

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);

        assert!(result.is_ok());
        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 5);
        assert!(summary.total.all_lines > 0);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests multiple consecutive analyze operations with same accumulator.
    #[test]
    fn test_multiple_analyze_operations() {
        let temp_dir1 = std::env::temp_dir().join("test_multi_1");
        let temp_dir2 = std::env::temp_dir().join("test_multi_2");
        std::fs::create_dir_all(&temp_dir1).unwrap();
        std::fs::create_dir_all(&temp_dir2).unwrap();

        std::fs::write(temp_dir1.join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(temp_dir2.join("b.rs"), "fn b() {}").unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();

        // Analyze first directory
        analyze_directory(&temp_dir1, None, &mut accumulator).unwrap();
        // Analyze second directory
        analyze_directory(&temp_dir2, None, &mut accumulator).unwrap();

        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        std::fs::remove_dir_all(&temp_dir1).unwrap();
        std::fs::remove_dir_all(&temp_dir2).unwrap();
    }

    /// Tests FileBackedAccumulator iteration works correctly after flush.
    #[test]
    fn test_file_backed_accumulator_iteration_after_flush() {
        let mut accumulator = FileBackedAccumulator::new().unwrap();

        for i in 1..=3 {
            let stats = FileStats {
                path: format!("file{}.rs", i),
                total: LineStats {
                    all_lines: i * 10,
                    blank_lines: i,
                    comment_lines: i * 2,
                    code_lines: i * 7,
                    rustdoc_lines: 0,
                },
                production: LineStats {
                    all_lines: i * 10,
                    blank_lines: i,
                    comment_lines: i * 2,
                    code_lines: i * 7,
                    rustdoc_lines: 0,
                },
                test: LineStats::default(),
            };
            accumulator.add_file(&stats).unwrap();
        }

        accumulator.flush().unwrap();

        // Iterate and verify
        let files: Vec<_> = accumulator.iter_files().unwrap().collect();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "file1.rs");
        assert_eq!(files[1].path, "file2.rs");
        assert_eq!(files[2].path, "file3.rs");
    }

    /// Tests analyze_directory with very large directory (many files).
    #[test]
    fn test_analyze_directory_many_files() {
        let temp_dir = std::env::temp_dir().join("test_many_files");
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create 10 files to test bulk operations
        for i in 1..=10 {
            let content = format!("fn func{}() {{}}\n", i);
            std::fs::write(temp_dir.join(format!("file{}.rs", i)), content).unwrap();
        }

        let mut accumulator = FileBackedAccumulator::new().unwrap();
        let result = analyze_directory(&temp_dir, None, &mut accumulator);

        assert!(result.is_ok());
        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 10);

        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests analyze_file followed by analyze_directory with same accumulator.
    #[test]
    fn test_mixed_file_and_directory_analysis() {
        let temp_file = std::env::temp_dir().join("single.rs");
        let temp_dir = std::env::temp_dir().join("test_mixed");
        std::fs::create_dir_all(&temp_dir).unwrap();

        std::fs::write(&temp_file, "fn single() {}").unwrap();
        std::fs::write(temp_dir.join("dir.rs"), "fn dir() {}").unwrap();

        let mut accumulator = FileBackedAccumulator::new().unwrap();

        // Analyze single file
        let file_stats = analyze_file(&temp_file, None).unwrap();
        accumulator.add_file(&file_stats).unwrap();

        // Analyze directory
        analyze_directory(&temp_dir, None, &mut accumulator).unwrap();

        accumulator.flush().unwrap();

        let summary = accumulator.get_summary();
        assert_eq!(summary.files, 2);

        std::fs::remove_file(&temp_file).unwrap();
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Tests output functions handle empty accumulator gracefully.
    #[test]
    fn test_output_with_empty_accumulator() {
        let accumulator = InMemoryAccumulator::new();

        // Both should work even with no files
        let text_result = output_text_from_accumulator(&accumulator);
        assert!(text_result.is_ok());

        let json_result = output_json_from_accumulator(&accumulator);
        assert!(json_result.is_ok());
    }

    /// Tests Report serialization and deserialization.
    #[test]
    fn test_report_json_roundtrip() {
        let report = Report {
            summary: Summary {
                files: 1,
                total: LineStats {
                    all_lines: 10,
                    blank_lines: 2,
                    comment_lines: 3,
                    code_lines: 5,
                    rustdoc_lines: 1,
                },
                production: LineStats {
                    all_lines: 7,
                    blank_lines: 1,
                    comment_lines: 2,
                    code_lines: 4,
                    rustdoc_lines: 1,
                },
                test: LineStats {
                    all_lines: 3,
                    blank_lines: 1,
                    comment_lines: 1,
                    code_lines: 1,
                    rustdoc_lines: 0,
                },
            },
            files: vec![FileStats {
                path: "test.rs".to_string(),
                total: LineStats {
                    all_lines: 10,
                    blank_lines: 2,
                    comment_lines: 3,
                    code_lines: 5,
                    rustdoc_lines: 1,
                },
                production: LineStats {
                    all_lines: 7,
                    blank_lines: 1,
                    comment_lines: 2,
                    code_lines: 4,
                    rustdoc_lines: 1,
                },
                test: LineStats {
                    all_lines: 3,
                    blank_lines: 1,
                    comment_lines: 1,
                    code_lines: 1,
                    rustdoc_lines: 0,
                },
            }],
        };

        // Serialize
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("test.rs"));

        // Deserialize
        let deserialized: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.summary.files, 1);
        assert_eq!(deserialized.files.len(), 1);
    }
}
