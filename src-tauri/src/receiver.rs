//! A live destination implemented in Rust: registers a session record and
//! queues batches the Console delivers. Used by the MCP server and `annotate wait`.
use crate::batch::{Package, MAX_BODY, MAX_ITEMS};
use crate::bridge::{self, Advertised, Record, Server};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Queue = Arc<(Mutex<(VecDeque<Package>, HashSet<String>)>, Condvar)>;

pub struct Receiver {
    pub record: Record,
    queue: Queue,
    _server: Server,
    _advertised: Advertised,
}

fn validate(value: Value) -> Result<Package, String> {
    let package: Package =
        serde_json::from_value(value).map_err(|e| format!("Invalid annotation batch: {e}"))?;
    if package.protocol != 1 || package.id.is_empty() || package.id.len() > 80 {
        return Err("Unsupported annotation batch".into());
    }
    if package.items.is_empty() || package.items.len() > MAX_ITEMS {
        return Err(format!("Send 1–{MAX_ITEMS} annotations"));
    }
    for item in &package.items {
        crate::batch::png_size(&item.image)?;
    }
    Ok(package)
}

impl Receiver {
    pub fn start(
        harness: &str,
        harness_name: &str,
        project: &str,
        name: &str,
        modes: Value,
    ) -> Result<Self, String> {
        let queue: Queue = Arc::new((
            Mutex::new((VecDeque::new(), HashSet::new())),
            Condvar::new(),
        ));
        let id = uuid::Uuid::new_v4().to_string();
        let session = uuid::Uuid::new_v4().to_string();
        let status_record = Arc::new(Mutex::new(None::<Record>));
        let (q, status, name, modes) = (
            queue.clone(),
            status_record.clone(),
            name.to_string(),
            modes.clone(),
        );
        let server = bridge::serve(
            MAX_BODY,
            Arc::new(move |req| match (req.method.as_str(), req.path.as_str()) {
                ("GET", "/session") => {
                    let record = status
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or((503, "Starting".to_string()))?;
                    Ok(crate::deliver::session_status(
                        &record,
                        &name,
                        modes.clone(),
                    ))
                }
                ("POST", "/deliver") => {
                    let package = validate(req.body).map_err(|e| (400, e))?;
                    let (lock, cv) = &*q;
                    let mut guard = lock.lock().unwrap();
                    let result = json!({ "ok": true, "count": package.items.len(), "savedTo": package.saved_to, "deliverAs": package.deliver_as });
                    if guard.1.insert(package.id.clone()) {
                        guard.0.push_back(package);
                        cv.notify_all();
                    }
                    Ok(result)
                }
                ("POST", "/status") => Ok(json!({ "ok": true })),
                _ => Err((404, "Not found".to_string())),
            }),
        )?;
        let record = Record {
            id,
            url: server.url.clone(),
            token: server.token.clone(),
            harness: harness.into(),
            harness_name: harness_name.into(),
            project: project.into(),
            session,
            pid: std::process::id(),
            protocol: 1,
            legacy: false,
        };
        *status_record.lock().unwrap() = Some(record.clone());
        let advertised = bridge::advertise(&record, &crate::paths::sessions())?;
        Ok(Self {
            record,
            queue,
            _server: server,
            _advertised: advertised,
        })
    }

    /// Takes every queued batch, waiting up to `wait` for the first one.
    pub fn take(&self, wait: Duration) -> Vec<Package> {
        let (lock, cv) = &*self.queue;
        let deadline = Instant::now() + wait;
        let mut guard = lock.lock().unwrap();
        while guard.0.is_empty() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            guard = cv.wait_timeout(guard, deadline - now).unwrap().0;
        }
        guard.0.drain(..).collect()
    }
}

/// Project folder for a receiver started in `cwd` (root/home are not projects).
pub fn project_for(cwd: &std::path::Path) -> String {
    let home = dirs::home_dir();
    if cwd.parent().is_none() || home.as_deref() == Some(cwd) {
        String::new()
    } else {
        cwd.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Draft, Mark};
    use base64::Engine;

    #[test]
    fn console_delivers_to_receiver() {
        let home = std::env::temp_dir().join(format!("annotate-e2e-{}", uuid::Uuid::new_v4()));
        std::env::set_var("ANNOTATE_REGISTRY_DIR", home.join("sessions"));
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let receiver = Receiver::start(
            "claude-code",
            "Claude Code",
            project.to_str().unwrap(),
            "test",
            json!([{ "id": "queue", "label": "Queue" }]),
        )
        .unwrap();

        // The Console discovers the live session…
        let live = crate::harness::live_sessions();
        let (record, destination) = live
            .into_iter()
            .find(|(r, _)| r.id == receiver.record.id)
            .expect("session discovered");
        assert_eq!(destination.harness_name, "Claude Code");
        assert_eq!(destination.modes[0].id, "queue");

        // …and sends a batch to it.
        let png = crate::batch::encode_png(&image::RgbaImage::from_pixel(
            8,
            6,
            image::Rgba([0, 128, 255, 255]),
        ));
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let draft = Draft {
            id: "d1".into(),
            image: b64,
            base: None,
            preview: String::new(),
            comment: "Rename this button".into(),
            kind: "rectangle".into(),
            points: vec![[0.1, 0.1], [0.6, 0.6]],
            marks: vec![Mark {
                kind: "rectangle".into(),
                points: vec![[0.1, 0.1], [0.6, 0.6]],
                comment: "Rename this button".into(),
            }],
            width: 8,
            height: 6,
            source: "App · whole application".into(),
        };
        let settings = crate::settings::Settings::default();
        let outcome = crate::deliver::deliver(
            &[draft.clone()],
            "batch-e2e-0001",
            &destination,
            Some(&record),
            "queue",
            &settings,
        )
        .unwrap();
        assert!(outcome.saved_to.contains(".annotate"));
        // Retries of the same batch are idempotent.
        crate::deliver::deliver(
            &[draft],
            "batch-e2e-0001",
            &destination,
            Some(&record),
            "queue",
            &settings,
        )
        .unwrap();

        let packages = receiver.take(Duration::from_secs(2));
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].items[0].comment, "Rename this button");
        assert!(packages[0].text.contains("UI annotation batch (1 item)"));
        let content = crate::mcp::package_content(&packages);
        assert_eq!(content.iter().filter(|c| c["type"] == "image").count(), 1);
        drop(receiver);
        assert!(crate::bridge::records(&home.join("sessions")).is_empty());
        std::fs::remove_dir_all(home).unwrap();
    }
}
