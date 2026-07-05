//! Authenticated GET /download/{filename}.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use tokio::fs;
use tracing::{info, warn};

use crate::state::AppState;
use crate::temp_files::{is_safe_filename, resolve_temp_file};

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub device_id: Option<String>,
}

pub async fn download_handler(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !is_authorized(&state, &query, &headers).await {
        warn!("Unauthorized download attempt for: {}", filename);
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    if !is_safe_filename(&filename) {
        warn!("Blocked unsafe filename: {}", filename);
        return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
    }

    let files_dir = &state.config.files_dir;
    let filepath_canonical = match resolve_temp_file(files_dir, &filename) {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };

    match fs::metadata(&filepath_canonical).await {
        Ok(metadata) if metadata.is_file() => match fs::read(&filepath_canonical).await {
            Ok(contents) => {
                info!(
                    "Served file: {} ({})",
                    filename,
                    crate::utils::format_bytes(contents.len() as u64)
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/octet-stream")
                    .header(
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{filename}\""),
                    )
                    .body(Body::from(contents))
                    .unwrap()
                    .into_response()
            }
            Err(e) => {
                warn!("Failed to read {:?}: {}", filepath_canonical, e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response()
            }
        },
        Ok(_) => (StatusCode::NOT_FOUND, "Not a file").into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "File not found").into_response(),
    }
}

async fn is_authorized(state: &AppState, query: &DownloadQuery, headers: &HeaderMap) -> bool {
    let from_header = headers
        .get("x-device-id")
        .and_then(|v| v.to_str().ok());
    let from_query = query.device_id.as_deref();
    let provided = from_header.or(from_query);
    state.device_id_matches(provided).await
}
