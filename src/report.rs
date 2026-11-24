//! Read strategy-writable artifacts and display them in an opaque browser frame.
use crate::{ApiError, backtests, bad, store};
use axum::{
    extract::Path as UrlPath,
    http::{StatusCode, header},
    response::{Html, IntoResponse},
};
use rustix::fs::{Mode, OFlags, openat};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

// The srcdoc frame inherits this policy. In particular, default-src also
// blocks frame navigation to network URLs, while Plotly runs inline.
const POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; \
    style-src 'unsafe-inline'; img-src data: blob:; base-uri 'none'; \
    form-action 'none'; frame-ancestors 'self'";

fn open_artifact(runs: &Path, id: &str) -> io::Result<File> {
    let root = File::open(runs)?;
    let run = openat(
        &root,
        id,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let artifact = File::from(openat(
        &run,
        "report.html",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    if !artifact.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    Ok(artifact)
}

pub fn available(id: &str) -> bool {
    backtests::valid_id(id) && open_artifact(&store::runs_dir(), id).is_ok()
}

fn preview(source: &str) -> String {
    let mut html = String::with_capacity(source.len());
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Backtest report</title>\
         <style>html,body{margin:0;height:100%}iframe{display:block;width:100%;height:100%;border:0}</style>\
         </head><body><iframe title=\"Backtest chart\" sandbox=\"allow-scripts\" srcdoc=\"",
    );
    for ch in source.chars() {
        match ch {
            '&' => html.push_str("&amp;"),
            '"' => html.push_str("&quot;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            _ => html.push(ch),
        }
    }
    html.push_str("\"></iframe></body></html>");
    html
}

pub async fn show(UrlPath(id): UrlPath<String>) -> Result<impl IntoResponse, ApiError> {
    if !backtests::valid_id(&id) {
        return Err(bad(StatusCode::BAD_REQUEST, "bad run id"));
    }
    let report = tokio::task::spawn_blocking(move || {
        let mut source = String::new();
        open_artifact(&store::runs_dir(), &id)?.read_to_string(&mut source)?;
        Ok::<_, io::Error>(preview(&source))
    })
    .await
    .map_err(|_| bad(StatusCode::INTERNAL_SERVER_ERROR, "could not load report"))?
    .map_err(|_| bad(StatusCode::NOT_FOUND, "report unavailable"))?;

    Ok((
        [
            (header::CONTENT_SECURITY_POLICY, POLICY),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(report),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, time::SystemTime};

    #[test]
    fn report_artifact_must_be_a_regular_file_inside_a_real_run_directory() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runs = std::env::temp_dir().join(format!("quantrig-report-{suffix}"));
        let run = runs.join("123");
        let report = run.join("report.html");
        fs::create_dir_all(&run).unwrap();
        fs::write(&report, "<p>Chart</p>").unwrap();
        assert!(open_artifact(&runs, "123").is_ok());

        symlink(&run, runs.join("456")).unwrap();
        assert!(open_artifact(&runs, "456").is_err());

        let other = runs.join("other.html");
        fs::write(&other, "outside run").unwrap();
        fs::remove_file(&report).unwrap();
        symlink(&other, &report).unwrap();
        assert!(open_artifact(&runs, "123").is_err());

        fs::remove_file(&report).unwrap();
        fs::create_dir(&report).unwrap();
        assert!(open_artifact(&runs, "123").is_err());
        fs::remove_dir_all(runs).unwrap();
    }

    #[test]
    fn preview_escapes_report_markup_inside_the_sandboxed_attribute() {
        let html = preview(r#""><h1>Research & results</h1>"#);
        assert!(html.contains(
            "sandbox=\"allow-scripts\" srcdoc=\"&quot;&gt;&lt;h1&gt;Research &amp; results&lt;/h1&gt;\""
        ));
    }
}
