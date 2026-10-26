//! Der Weg zum Server.
//!
//! Blockierend und ohne Laufzeitumgebung: ein Kommandozeilenwerkzeug
//! macht einen Aufruf und wartet darauf. Ein Tokio-Unterbau dafür wäre
//! ein Megabyte Binär und eine Abhängigkeit mehr, für einen Nebenläufer,
//! den es nicht gibt.

use std::time::Duration;

use crate::exit::{Code, Failure, Outcome, from_status};

pub struct Client {
    inner: reqwest::blocking::Client,
    pub server: String,
    token: Option<String>,
}

/// Was vom Server zurückkam.
pub struct Response {
    pub status: u16,
    pub body: serde_json::Value,
    /// Die Bytes, wie sie kamen.
    ///
    /// **Ein Dutzend Wege in Rakete antwortet nicht mit JSON**, sondern
    /// mit einem PDF, einer XML-Rechnung, einem SVG oder einem Foto. Ohne
    /// diese Bytes hätte das Werkzeug versucht, ein PDF als Text zu
    /// deuten, und der Bildschirm wäre voll Unrat gewesen.
    pub bytes: Vec<u8>,
    /// Woran man erkennt, ob das JSON ist. Leer, wenn der Server nichts
    /// gesagt hat.
    pub content_type: String,
    /// The server's own headers, names in lower case.
    ///
    /// A page image says in `x-page-count` how many pages there are; the
    /// bytes cannot say it, and without it an agent would not know there
    /// is a second page to look at.
    pub headers: Vec<(String, String)>,
}

impl Response {
    /// Ob die Antwort JSON ist, und damit, ob `body` etwas taugt.
    ///
    /// Am Kopf des Servers und nicht daran, ob sich der Inhalt zufällig
    /// lesen lässt: die ersten Bytes eines PDF sind gültiger Text, und
    /// ein leeres JSON-Objekt sieht aus wie eine leere Datei.
    pub fn is_json(&self) -> bool {
        self.content_type.is_empty() || self.content_type.contains("json")
    }

    /// One header by name, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The content type to send a file under.
///
/// **What the bytes are, not what the name says**, because the server
/// checks the bytes: a logo upload declares `image/png` and also takes an
/// SVG, and sent under the wrong type it is refused. Where the server
/// declares nothing particular, the bytes go as they are and the server
/// looks at them itself.
pub fn file_content_type(bytes: &[u8], declared: &str) -> String {
    if declared.is_empty() || declared == "application/octet-stream" {
        return "application/octet-stream".to_string();
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if bytes.starts_with(b"\x89PNG") {
        "image/png".into()
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg".into()
    } else if bytes.starts_with(b"%PDF") {
        "application/pdf".into()
    } else if head.contains("<svg") {
        "image/svg+xml".into()
    } else {
        declared.to_string()
    }
}

impl Client {
    pub fn new(server: &str, token: Option<String>) -> Outcome<Self> {
        let inner = reqwest::blocking::Client::builder()
            // Dreißig Sekunden: ein Bericht über ein Jahr darf rechnen,
            // ein hängender Aufruf darf keine Kette blockieren.
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("l3pus-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Failure::unreachable(format!("Kein Netzzugang: {error}")))?;
        Ok(Self {
            inner,
            server: server.to_string(),
            token,
        })
    }

    /// Ein Aufruf, ohne Deutung des Ergebnisses.
    ///
    /// **Ein Fehlschlag am Netz ist etwas anderes als eine Absage vom
    /// Server.** Das erste ist „ich konnte nicht nachsehen", das zweite
    /// eine Antwort. Deshalb landet nur das erste hier als `Err`.
    pub fn call(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<&serde_json::Value>,
    ) -> Outcome<Response> {
        self.send(method, path, query, body, None)
    }

    /// Derselbe Aufruf, aber mit rohen Bytes im Rumpf.
    ///
    /// **Nicht jeder Rumpf ist JSON.** Ein hochgeladenes Papier ist eine
    /// Datei (ADR-038), und sie durch eine Zeichenkette zu schicken
    /// hieße, sie vorher zu verstümmeln. Getrennte Funktion und kein
    /// zweiter Zweig im Aufrufer: die beiden schließen sich aus.
    pub fn call_with_file(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Outcome<Response> {
        self.send(
            method,
            path,
            query,
            None,
            Some((bytes, content_type.to_string())),
        )
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<&serde_json::Value>,
        file: Option<(Vec<u8>, String)>,
    ) -> Outcome<Response> {
        let url = format!("{}{}", self.server, path);
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| Failure::usage(format!("{method} ist keine Methode")))?;
        let mut request = self.inner.request(method, &url);
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some((bytes, content_type)) = file {
            request = request
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(bytes);
        }

        let response = request.send().map_err(|error| {
            Failure::unreachable(format!("{url} war nicht erreichbar: {error}"))
        })?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().ok()?.to_string(),
                ))
            })
            .collect();
        let bytes = response.bytes().map(|b| b.to_vec()).unwrap_or_default();
        // Ein leerer Rumpf ist gültig: 204 hat keinen.
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            match std::str::from_utf8(&bytes) {
                Ok(text) if !text.trim().is_empty() => serde_json::from_str(text)
                    .unwrap_or_else(|_| serde_json::Value::String(text.to_string())),
                _ => serde_json::Value::Null,
            }
        };
        Ok(Response {
            status,
            body,
            bytes,
            content_type,
            headers,
        })
    }
}

/// Was beim Nachfragen herauskam.
pub enum Conditional {
    /// Der Server sagt: unverändert. Null Bytes über die Leitung.
    Unchanged,
    /// Etwas Neues, samt der Marke dazu.
    Fresh {
        document: serde_json::Value,
        mark: Option<String>,
    },
}

impl Client {
    /// Nachfragen, ob sich etwas geändert hat, ohne es zu holen.
    ///
    /// **Der ganze Sinn ist die 304.** Ein Werkzeug, das seine Befehle
    /// beim Server holt, muss regelmäßig nachsehen, sonst bietet es
    /// Befehle an, die es nicht mehr gibt, und keine, die es längst
    /// gibt. Ohne Marke kostet jede dieser Proben die ganze
    /// Beschreibung; mit Marke kostet sie nichts, und deshalb darf man
    /// oft fragen.
    pub fn conditional_get(
        &self,
        path: &str,
        headers: &[(String, String)],
    ) -> Outcome<Conditional> {
        let url = format!("{}{}", self.server, path);
        let mut request = self.inner.get(&url);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|error| {
            Failure::unreachable(format!("{url} war nicht erreichbar: {error}"))
        })?;
        if response.status().as_u16() == 304 {
            return Ok(Conditional::Unchanged);
        }
        let mark = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = response
            .text()
            .map_err(|error| Failure::unreachable(error.to_string()))?;
        let document = serde_json::from_str(&text).map_err(|error| {
            Failure::unreachable(format!("{url} antwortete nicht mit JSON: {error}"))
        })?;
        Ok(Conditional::Fresh { document, mark })
    }
}

impl Client {
    /// Ein Aufruf, dessen Antwort nur im Erfolgsfall etwas taugt.
    pub fn expect_ok(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        body: Option<&serde_json::Value>,
    ) -> Outcome<serde_json::Value> {
        let response = self.call(method, path, query, body)?;
        match from_status(response.status) {
            Code::Ok => Ok(response.body),
            code => Err(Failure {
                code,
                message: complain(response.status, &response.body),
            }),
        }
    }
}

/// Aus der Antwort des Servers einen Satz machen, den ein Mensch liest.
///
/// Rakete antwortet mit `{"fehler": "..."}`; andere Server machen es
/// anders, und ein nackter Statuscode ist als Auskunft wertlos. Deshalb
/// erst nach dem Satz suchen und ihn sonst selbst bauen.
pub fn complain(status: u16, body: &serde_json::Value) -> String {
    for key in ["fehler", "error", "message", "detail"] {
        if let Some(text) = body.get(key).and_then(|v| v.as_str()) {
            return format!("{text} (HTTP {status})");
        }
    }
    if let Some(text) = body.as_str() {
        return format!("{text} (HTTP {status})");
    }
    match status {
        401 => "Nicht angemeldet. `rakete login`, oder RAKETE_TOKEN setzen.".into(),
        403 => "Dafür fehlt die Berechtigung.".into(),
        404 => "Das gibt es nicht.".into(),
        _ => format!("Der Server antwortete mit HTTP {status}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_servers_own_sentence_wins() {
        let text = complain(422, &json!({"fehler": "Gewerbekunden brauchen eine Firma"}));
        assert!(
            text.starts_with("Gewerbekunden brauchen eine Firma"),
            "{text}"
        );
        assert!(text.contains("422"), "{text}");
    }

    #[test]
    fn a_silent_server_still_gets_a_sentence() {
        // Ein nackter Statuscode ist als Auskunft wertlos.
        let text = complain(401, &json!(null));
        assert!(text.contains("rakete login"), "{text}");
    }

    fn antwort(content_type: &str) -> Response {
        Response {
            status: 200,
            body: serde_json::Value::Null,
            bytes: Vec::new(),
            content_type: content_type.into(),
            headers: Vec::new(),
        }
    }

    #[test]
    fn a_file_goes_out_as_what_it_is_where_the_server_cares() {
        // The logo route declares image/png and also takes SVG; sent as
        // PNG, an SVG is refused.
        let svg = b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert_eq!(file_content_type(svg, "image/png"), "image/svg+xml");
        assert_eq!(file_content_type(b"\x89PNG\r\n", "image/png"), "image/png");
        // Where the server takes any bytes, nothing is guessed.
        assert_eq!(
            file_content_type(b"%PDF-1.7", "application/octet-stream"),
            "application/octet-stream"
        );
        assert_eq!(
            file_content_type(b"%PDF-1.7", ""),
            "application/octet-stream"
        );
    }

    #[test]
    fn a_pdf_is_not_json() {
        // Der Fall, für den es die Unterscheidung gibt: die ersten Bytes
        // eines PDF sind lesbarer Text, und ohne den Kopf hielte man sie
        // für eine Antwort.
        assert!(!antwort("application/pdf").is_json());
        assert!(!antwort("image/jpeg").is_json());
        assert!(!antwort("application/xml").is_json());
        assert!(!antwort("text/csv; charset=windows-1252").is_json());
    }

    #[test]
    fn json_in_all_its_spellings_is_json() {
        assert!(antwort("application/json").is_json());
        assert!(antwort("application/json; charset=utf-8").is_json());
        assert!(antwort("application/problem+json").is_json());
    }

    #[test]
    fn a_server_that_says_nothing_is_taken_for_json() {
        // Ohne Kopf ist JSON die richtige Annahme: alles andere in
        // Rakete sagt ausdrücklich, was es ist.
        assert!(antwort("").is_json());
    }

    #[test]
    fn a_foreign_shape_is_still_read() {
        let text = complain(400, &json!({"message": "bad request"}));
        assert!(text.starts_with("bad request"), "{text}");
    }
}
