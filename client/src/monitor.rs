//! Ratatui dashboard: connections, task status, run counters.

use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;
use sqlx::SqlitePool;

use crate::db::TaskRunStats;
use crate::models::connection::Connection;
use crate::models::task::Task;
use crate::models::task_run::TaskRun;
use crate::ui;
use crate::validation;

struct TaskRow {
    task: Task,
    last_run: Option<TaskRun>,
    last_download_bytes: Option<i64>,
    stats: TaskRunStats,
}

struct ConnectionRow {
    conn: Connection,
    activity: &'static str,
    activity_style: Style,
    tasks: Vec<TaskRow>,
}

struct Dashboard {
    connections: Vec<ConnectionRow>,
    updated_at: DateTime<Utc>,
}

const RECENT_ACTIVITY: Duration = Duration::from_secs(30 * 60);
const EMPTY_ROW: [&str; 8] = ["", "", "", "", "", "", "", ""];

pub async fn run(pool: SqlitePool) -> anyhow::Result<()> {
    let mut terminal = setup_terminal()?;
    let mut force_refresh = true;
    let refresh_interval = Duration::from_secs(10);
    let mut last_refresh = std::time::Instant::now() - refresh_interval;
    let mut dashboard = Dashboard {
        connections: Vec::new(),
        updated_at: Utc::now(),
    };

    let result = loop {
        if force_refresh || last_refresh.elapsed() >= refresh_interval {
            dashboard = load_dashboard(&pool).await?;
            force_refresh = false;
            last_refresh = std::time::Instant::now();
        }

        if terminal.draw(|f| render(f, &dashboard)).is_err() {
            break Ok(());
        }

        let poll_ms = refresh_interval
            .saturating_sub(last_refresh.elapsed())
            .min(Duration::from_millis(250))
            .as_millis() as u64;

        if event::poll(Duration::from_millis(poll_ms.max(100))).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break Ok(());
                    }
                    KeyCode::Char('r') => force_refresh = true,
                    _ => {}
                }
            }
        }
    };

    restore_terminal()?;
    result
}

async fn load_dashboard(pool: &SqlitePool) -> anyhow::Result<Dashboard> {
    let connections = crate::db::list_all_connections(pool).await?;
    let mut rows = Vec::new();

    for conn in connections {
        if !conn.enabled {
            continue;
        }
        let tasks = crate::db::list_tasks_for_connection(pool, &conn.id).await?;
        let mut task_rows = Vec::new();

        for task in tasks.into_iter().filter(|t| t.enabled) {
            let last_run = crate::db::get_latest_run_for_task(pool, &task.id).await?;
            let last_download_bytes =
                crate::db::get_latest_download_bytes_for_task(pool, &task.id).await?;
            let stats = crate::db::get_task_run_stats(pool, &task.id).await?;
            task_rows.push(TaskRow {
                task,
                last_run,
                last_download_bytes,
                stats,
            });
        }

        let (activity, activity_style) = connection_activity(&task_rows);
        rows.push(ConnectionRow {
            conn,
            activity,
            activity_style,
            tasks: task_rows,
        });
    }

    Ok(Dashboard {
        connections: rows,
        updated_at: Utc::now(),
    })
}

fn connection_activity(tasks: &[TaskRow]) -> (&'static str, Style) {
    let latest = tasks
        .iter()
        .filter_map(|t| t.last_run.as_ref())
        .map(|r| r.run_at)
        .max();

    match latest {
        None => ("NO RUNS", Style::default().fg(Color::DarkGray)),
        Some(t) if Utc::now().signed_duration_since(t).to_std().unwrap_or(RECENT_ACTIVITY)
            < RECENT_ACTIVITY =>
        {
            ("RECENT", Style::default().fg(Color::Green))
        }
        Some(_) => ("IDLE", Style::default().fg(Color::Yellow)),
    }
}

fn render(f: &mut Frame, dashboard: &Dashboard) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(f.area());

    let updated = dashboard
        .updated_at
        .with_timezone(&Local)
        .format("%H:%M:%S")
        .to_string();

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " Backup Monitor ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  data refreshed {updated}  read-only/WAL  ")),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw("=quit  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw("=refresh"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Monitor"),
    );
    f.render_widget(header, chunks[0]);

    let mut table_rows: Vec<Row> = Vec::new();

    for conn_row in &dashboard.connections {
        table_rows.push(Row::new(empty_cells()));

        let (ok_count, total) = count_ok(&conn_row.tasks);
        let (fail_sum, warn_sum) = sum_stats(&conn_row.tasks);

        let conn_label = format!(
            "{} {}",
            ui::server_addr_from_url(&conn_row.conn.url),
            conn_row.conn.slug
        );

        let (fail_text, fail_style) = count_cell(fail_sum, Color::Red);
        let (warn_text, warn_style) = count_cell(warn_sum, Color::Yellow);

        table_rows.push(Row::new(vec![
            Cell::from(conn_label).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(conn_row.activity).style(conn_row.activity_style),
            Cell::from(format!("{ok_count}/{total}")),
            Cell::from(""),
            Cell::from(""),
            Cell::from(fail_text).style(fail_style),
            Cell::from(warn_text).style(warn_style),
            Cell::from(""),
        ]));

        for task_row in &conn_row.tasks {
            let (result_text, result_style) =
                last_status(&task_row.last_run, &task_row.task.task_type);
            let size = last_size(
                &task_row.last_run,
                task_row.last_download_bytes,
                &task_row.task.task_type,
            );
            let (fail_text, fail_style) = count_cell(task_row.stats.fail_count, Color::Red);
            let (warn_text, warn_style) = count_cell(task_row.stats.warn_count, Color::Yellow);
            let time = task_row
                .last_run
                .as_ref()
                .map(|r| r.run_at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".into());

            table_rows.push(Row::new(vec![
                Cell::from(format!("  {}", task_row.task.task_name)),
                Cell::from(validation::normalize_task_type(&task_row.task.task_type)),
                Cell::from(task_row.task.schedule.as_str()),
                Cell::from(result_text).style(result_style),
                Cell::from(size),
                Cell::from(fail_text).style(fail_style),
                Cell::from(warn_text).style(warn_style),
                Cell::from(time),
            ]));
        }
    }

    if table_rows.is_empty() {
        let mut cells = empty_cells();
        cells[0] = Cell::from("No enabled connections/tasks");
        table_rows.push(Row::new(cells));
    }

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(26),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(7),
        ],
    )
    .header(
        Row::new(vec![
            "CONNECTION / TASK",
            "ACTIVITY / TYPE",
            "OK / SCHEDULE",
            "LAST",
            "SIZE",
            "FAIL",
            "WARN",
            "LAST RUN",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Connections & Tasks"),
    );
    f.render_widget(table, chunks[1]);
}

fn empty_cells() -> Vec<Cell<'static>> {
    EMPTY_ROW.iter().map(|s| Cell::from(*s)).collect()
}

fn sum_stats(tasks: &[TaskRow]) -> (i64, i64) {
    tasks.iter().fold((0, 0), |(f, w), t| {
        (f + t.stats.fail_count, w + t.stats.warn_count)
    })
}

fn count_cell(n: i64, active: Color) -> (String, Style) {
    if n == 0 {
        ("—".into(), Style::default().fg(Color::DarkGray))
    } else {
        (n.to_string(), Style::default().fg(active))
    }
}

fn count_ok(tasks: &[TaskRow]) -> (usize, usize) {
    let total = tasks.len();
    let ok = tasks
        .iter()
        .filter(|t| {
            t.last_run
                .as_ref()
                .map(|r| r.status == "ok" || r.status == "warn")
                .unwrap_or(false)
        })
        .count();
    (ok, total)
}

fn is_dir_sync_pass(last_run: &TaskRun, task_type: &str) -> bool {
    validation::normalize_task_type(task_type) == "dir_sync"
        && last_run.status == "ok"
        && last_run.file_size_bytes.unwrap_or(-1) == 0
}

/// Last run status. OK/WARN = download + SHA256 passed; FAIL = server/download error.
fn last_status(last_run: &Option<TaskRun>, task_type: &str) -> (String, Style) {
    match last_run {
        None => ("—".into(), Style::default().fg(Color::DarkGray)),
        Some(r) if is_dir_sync_pass(r, task_type) => {
            ("pass".into(), Style::default().fg(Color::Cyan))
        }
        Some(r) if r.status == "ok" => ("OK ✓".into(), Style::default().fg(Color::Green)),
        Some(r) if r.status == "warn" => ("OK ✓".into(), Style::default().fg(Color::Yellow)),
        Some(r) if r.status == "fail" => {
            let msg = r.error.as_deref().unwrap_or("fail");
            (
                truncate(msg, 8),
                Style::default().fg(Color::Red),
            )
        }
        Some(_) => ("ERR".into(), Style::default().fg(Color::Red)),
    }
}

fn last_size(
    last_run: &Option<TaskRun>,
    last_download_bytes: Option<i64>,
    task_type: &str,
) -> String {
    match last_run {
        None => "—".into(),
        Some(r) if is_dir_sync_pass(r, task_type) => last_download_bytes
            .map(format_size)
            .unwrap_or_else(|| "0 B".into()),
        Some(r) => r
            .file_size_bytes
            .map(format_size)
            .unwrap_or_else(|| "—".into()),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn format_size(bytes: i64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{} B", bytes)
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

fn setup_terminal() -> anyhow::Result<ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    Ok(ratatui::Terminal::new(backend)?)
}

fn restore_terminal() -> anyhow::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    Ok(())
}
