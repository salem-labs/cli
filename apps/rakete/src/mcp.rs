//! Derselbe Zugang für Claude Desktop und Claude Code (ADR-034).
//!
//! **Fünf Werkzeuge und nicht zweihundertachtundneunzig.** Das ist die
//! eine Entscheidung, auf die es hier ankommt. Jede Anleitung zu großen
//! Schnittstellen warnt vor demselben: hunderte Werkzeugbeschreibungen
//! fressen den Kontext, und je mehr davon nebeneinanderstehen, desto
//! schlechter wird die Auswahl. Ein Modell, das aus dreihundert fast
//! gleich klingenden Namen wählen muss, wählt schlechter als eines, das
//! erst sucht und dann aufruft.
//!
//! Also: `groups` für den Überblick, `search` zum Finden, `operations`
//! für die Einzelheiten einer Gruppe, `call` zum Ausführen. Die ersten
//! drei kosten zusammen weniger Kontext als zehn echte Werkzeuge, und
//! sie reichen, um den vierten richtig zu benutzen.
//!
//! `describe` came fifth, on 26.10.2026: one command in full, with its
//! description and the whole body schema. `operations` stays a short list
//! on purpose, because a group of thirty commands with their schemas
//! would cost more context than the five tools together.
//!
//! **Die Werkzeuge stehen fest, die Befehle nicht.** Was `call`
//! ausführen kann, kommt aus der Beschreibung des Servers, genau wie bei
//! der Kommandozeile. Ein neuer Endpunkt in Rakete ist hier sofort da,
//! ohne dass jemand dieses Programm anfasst.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Deserialize;

use l3pus_cli::api::Client;
use l3pus_cli::exit::{Code, Outcome};

use crate::spec::Catalog;

/// Der Dienst, den Claude und andere Werkzeuge ansprechen.
///
/// **Der Katalog ist tauschbar und nicht fest.** Bis zum 20.09.2026 las
/// dieser Dienst die Beschreibung des Servers einmal beim Start und
/// hielt sie für die ganze Laufzeit. Ein Plugin in einer
/// Schreibtischanwendung läuft tagelang; wurde in der Zeit ein neuer
/// Befehl ausgerollt, gab es ihn hier nicht, und der einzige Ausweg war,
/// die ganze Anwendung neu zu starten. Genau das ist am 20.09.2026
/// passiert: der Server kannte sieben Mandanten-Befehle, das Plugin
/// vier.
///
/// Das Protokoll sieht für so etwas `notifications/tools/list_changed`
/// vor, und das hilft hier nicht: wir bieten vier feste Werkzeuge an,
/// der Befehlsbaum steckt als Daten darin. Die Werkzeugliste ändert
/// sich nie, es gäbe also nichts zu melden. Also frischt der Dienst
/// sich selbst auf.
#[derive(Clone)]
pub struct Rakete {
    catalog: Arc<std::sync::RwLock<Arc<Catalog>>>,
    /// Wann zuletzt nachgeholt wurde. Beginnt beim Start, denn da war
    /// der Befehlsbaum gerade frisch.
    last_refresh: Arc<std::sync::Mutex<std::time::Instant>>,
    server: String,
    token: Option<String>,
}

/// Wie lange nach einem Nachholen nicht wieder nachgeholt wird.
///
/// **Ohne das kostet jeder Vertipper 363 KB.** Ein Modell, das nach
/// `kunden` statt `customers` fragt, hat keinen veralteten
/// Befehlsbaum vor sich, sondern danebengegriffen; aufgefallen ist es
/// im Test, der genau das tut und dabei den echten Server anrief.
///
/// Eine Minute, weil das die Größenordnung ist, in der ein Ausrollen
/// passiert: wer gerade eine neue Fassung hochgeschoben hat, wartet
/// gern sechzig Sekunden, und wer sich vertippt, merkt nichts davon.
const REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GroupParams {
    /// Der Name der Gruppe, etwa `customers` oder `dispatch`.
    pub group: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DescribeParams {
    /// Die Gruppe, etwa `themes`.
    pub group: String,
    /// Der Befehl innerhalb der Gruppe, etwa `add-version`.
    pub command: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Ein Wort, nach dem gesucht wird. Gesucht wird im Namen des
    /// Befehls, im Pfad und in der Beschreibung, deutsch wie englisch.
    pub term: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CallParams {
    /// Die Gruppe, etwa `customers`.
    pub group: String,
    /// Der Befehl innerhalb der Gruppe, etwa `list` oder `get`.
    pub command: String,
    /// Werte für die Platzhalter im Pfad, nach Namen. Für
    /// `/api/customers/{id}` also `{"id": "42"}`.
    #[serde(default)]
    pub path: Option<serde_json::Map<String, serde_json::Value>>,
    /// Werte, die hinten an die Adresse gehängt werden, nach Namen.
    #[serde(default)]
    pub query: Option<serde_json::Map<String, serde_json::Value>>,
    /// Der Rumpf, für alles Schreibende. Welche Felder es gibt und welche
    /// Werte sie annehmen, sagt `rakete_describe`.
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// Für Befehle, die eine Datei wollen (etwa das Logo): der Pfad einer
    /// Datei auf diesem Rechner.
    #[serde(default)]
    pub file: Option<String>,
    /// Dieselbe Datei als Base64, wenn es keinen Pfad gibt.
    #[serde(default)]
    pub file_base64: Option<String>,
}

#[tool_router]
impl Rakete {
    pub fn new(catalog: Arc<Catalog>, server: String, token: Option<String>) -> Self {
        // Der Router wird vom Makro bei Bedarf gebaut und nicht hier
        // gehalten: eine Kopie davon im Zustand wäre eine zweite
        // Wahrheit über dieselben fünf Werkzeuge.
        Self {
            catalog: Arc::new(std::sync::RwLock::new(catalog)),
            last_refresh: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
            server,
            token,
        }
    }

    /// Der Befehlsbaum, wie er gerade gilt.
    fn catalog(&self) -> Arc<Catalog> {
        self.catalog
            .read()
            .map(|guard| guard.clone())
            // Ein vergifteter Riegel heißt, dass irgendwo eine Panik
            // war. Dann lieber mit dem arbeiten, was dasteht, als den
            // Dienst darüber sterben zu lassen: er ist das Bindeglied
            // zu allem anderen.
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Die Beschreibung neu holen, weil etwas gefehlt hat.
    ///
    /// **Geholt und nicht gefragt**, aus demselben Grund wie in der
    /// Kommandozeile: die Marke beschreibt, was der Server hat, und
    /// wenn ein Befehl fehlt, ist die Annahme über das, was bei uns
    /// liegt, gerade widerlegt. Eine Antwort „unverändert" ließe uns
    /// mit derselben Lücke zurück.
    ///
    /// Gibt zurück, ob sich etwas geändert hat, damit der Aufrufer es
    /// noch einmal versuchen kann statt zweimal zu holen.
    fn refresh_catalog(&self) -> bool {
        {
            let mut last = match self.last_refresh.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if last.elapsed() < REFRESH_COOLDOWN {
                return false;
            }
            // Vor dem Holen vermerkt und nicht danach: scheitert der
            // Aufruf, soll trotzdem eine Minute Ruhe sein. Ein Server,
            // der nicht antwortet, antwortet in der nächsten Sekunde
            // meistens auch nicht.
            *last = std::time::Instant::now();
        }
        let Ok(client) = Client::new(&self.server, None) else {
            return false;
        };
        let Ok(l3pus_cli::api::Conditional::Fresh { document, .. }) =
            client.conditional_get("/api/openapi.json", &[])
        else {
            return false;
        };
        let Ok(fresh) = crate::spec::parse(&document) else {
            return false;
        };
        let changed = fresh.count() != self.catalog().count();
        if let Ok(mut guard) = self.catalog.write() {
            *guard = Arc::new(fresh);
        }
        changed
    }

    #[tool(
        description = "Die Bereiche dieses Rakete-Servers mit der Anzahl ihrer Befehle. Der Einstieg: erst hier nachsehen, welcher Bereich gemeint ist, dann rakete_operations oder rakete_search."
    )]
    fn rakete_groups(&self) -> String {
        let list: Vec<_> = self
            .catalog()
            .groups
            .iter()
            .map(|(name, operations)| {
                serde_json::json!({
                    "group": name,
                    "commands": operations.len(),
                    // Ein paar Beispiele statt einer nackten Zahl: daran
                    // erkennt man den Bereich, ohne ihn erst zu öffnen.
                    "examples": operations
                        .values()
                        .take(3)
                        .map(|operation| operation.name.clone())
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        pretty(&serde_json::json!({
            "server": self.server,
            "groups": list,
        }))
    }

    #[tool(
        description = "Sucht Befehle über alle Bereiche hinweg, nach einem Wort im Namen, im Pfad oder in der Beschreibung. Der schnellste Weg, wenn klar ist, was gebraucht wird, aber nicht, wo es steht."
    )]
    fn rakete_search(&self, Parameters(SearchParams { term }): Parameters<SearchParams>) -> String {
        let needle = term.trim().to_lowercase();
        let mut hits = Vec::new();
        for (group, operations) in &self.catalog().groups {
            for operation in operations.values() {
                let haystack = format!(
                    "{} {} {} {}",
                    group, operation.name, operation.path, operation.summary
                )
                .to_lowercase();
                if haystack.contains(&needle) {
                    hits.push(serde_json::json!({
                        "group": group,
                        "command": operation.name,
                        "summary": operation.summary,
                        "method": operation.method,
                        "path": operation.path,
                    }));
                }
            }
        }
        // Gekappt, damit eine zu weite Suche nicht den halben Katalog
        // zurückgibt. Wer dreißig Treffer hat, hat das falsche Wort
        // gesucht.
        let total = hits.len();
        hits.truncate(30);
        pretty(&serde_json::json!({
            "found": total,
            "shown": hits.len(),
            "matches": hits,
        }))
    }

    #[tool(
        description = "Alle Befehle eines Bereichs in Kurzform: Pfadwerte, Abfragewerte, Pflichtfelder des Rumpfes, ob eine Datei verlangt wird und was zurückkommt. Für die ganze Beschreibung eines Befehls mit Ablauf und allen Feldern rakete_describe."
    )]
    fn rakete_operations(
        &self,
        Parameters(GroupParams { group }): Parameters<GroupParams>,
    ) -> String {
        // Der Katalog wird festgehalten, solange daraus gelesen wird:
        // er kann zwischendurch getauscht werden.
        //
        // **Fehlt die Gruppe, wird einmal nachgeholt.** Ein Plugin läuft
        // tagelang; zwischen seinem Start und dieser Frage kann längst
        // ausgerollt worden sein. Erst danach ist „gibt es nicht" eine
        // Auskunft und keine Vermutung.
        let mut catalog = self.catalog();
        if !catalog.groups.contains_key(&group) && self.refresh_catalog() {
            catalog = self.catalog();
        }
        let Some(operations) = catalog.groups.get(&group) else {
            return pretty(&serde_json::json!({
                "error": format!("Den Bereich {group} gibt es auf diesem Server nicht."),
                "groups": catalog.groups.keys().collect::<Vec<_>>(),
            }));
        };
        let list: Vec<_> = operations
            .values()
            .map(|operation| {
                serde_json::json!({
                    "command": operation.name,
                    "summary": operation.summary,
                    "method": operation.method,
                    "path": operation.path,
                    "path_values": operation.path_params.iter()
                        .map(|param| param.name.clone()).collect::<Vec<_>>(),
                    "query_values": operation.query_params.iter()
                        .map(|param| serde_json::json!({
                            "name": param.name,
                            "required": param.required,
                        })).collect::<Vec<_>>(),
                    "body": operation.has_body.then(|| if operation.wants_file {
                        serde_json::json!({ "file": true })
                    } else {
                        serde_json::json!({ "required_fields": operation.required_fields })
                    }),
                    "returns": operation.returns,
                })
            })
            .collect();
        pretty(&serde_json::json!({ "group": group, "commands": list }))
    }

    #[tool(
        description = "Ein Befehl in allen Einzelheiten: die ganze Beschreibung des Servers mit dem Ablauf (was vorher da sein muss, was danach kommt, was nicht umkehrbar ist), jeder Pfad- und Abfragewert, das vollständige Schema des Rumpfes mit Typen und erlaubten Werten, und was zurückkommt. Vor jedem schreibenden Aufruf hier nachsehen statt zu raten."
    )]
    fn rakete_describe(
        &self,
        Parameters(DescribeParams { group, command }): Parameters<DescribeParams>,
    ) -> String {
        let mut catalog = self.catalog();
        if catalog.find(&group, &command).is_none() && self.refresh_catalog() {
            catalog = self.catalog();
        }
        match catalog.find(&group, &command) {
            Some(operation) => pretty(&operation.detail()),
            None => pretty(&serde_json::json!({
                "error": format!("{group} {command} kennt dieser Server nicht."),
                "commands": catalog
                    .groups
                    .get(&group)
                    .map(|operations| operations.keys().cloned().collect::<Vec<_>>()),
            })),
        }
    }

    #[tool(
        description = "Führt einen Befehl aus und gibt die Antwort des Servers zurück. Bilder (etwa eine Seite als Vorschau) kommen als Bild zurück, andere Dateien als Pfad. Für Befehle, die eine Datei wollen, file oder file_base64 angeben. Lesende Befehle sind unbedenklich; alles Schreibende ändert echte Daten in einem Betrieb, also vorher fragen. Ob das Zeichen überhaupt schreiben darf, sagt der Server selbst."
    )]
    async fn rakete_call(&self, Parameters(params): Parameters<CallParams>) -> CallToolResult {
        match self.perform(params).await {
            Ok(result) => result,
            Err(failure) => {
                CallToolResult::error(vec![ContentBlock::text(pretty(&serde_json::json!({
                    "ok": false,
                    "reason": failure.message,
                    // Die Trennung, die auf der Kommandozeile die
                    // Rückgabewerte machen, steht hier als Wort: sonst hält
                    // ein Modell "nicht erreichbar" für "gibt es nicht" und
                    // erzählt es dem Menschen so weiter.
                    "kind": match failure.code {
                        Code::Wrong => "die Sache stimmt nicht, noch einmal ändert nichts",
                        Code::Usage => "der Aufruf war falsch zusammengesetzt",
                        Code::Unreachable => "wir konnten nicht nachsehen, die Frage ist offen",
                        Code::Ok => "in Ordnung",
                    },
                })))])
            }
        }
    }

    async fn perform(&self, params: CallParams) -> Outcome<CallToolResult> {
        let CallParams {
            group,
            command,
            path,
            query,
            body,
            file,
            file_base64,
        } = params;

        // Derselbe Weg wie oben, und hier zählt er am meisten: ein
        // Aufruf, der an einem veralteten Befehlsbaum scheitert, sieht
        // aus wie ein Fehler des Aufrufers.
        let operation = match self.catalog().find(&group, &command) {
            Some(operation) => operation.clone(),
            None => {
                self.refresh_catalog();
                self.catalog()
                    .find(&group, &command)
                    .ok_or_else(|| {
                        l3pus_cli::Failure::usage(format!(
                            "{group} {command} kennt dieser Server nicht. rakete_operations zeigt, was es gibt."
                        ))
                    })?
                    .clone()
            }
        };

        let values = path.unwrap_or_default();
        let mut path = operation.path.clone();
        for param in &operation.path_params {
            let wert = values.get(&param.name).map(as_text).ok_or_else(|| {
                l3pus_cli::Failure::usage(format!(
                    "Im Pfad fehlt {}. Erwartet werden: {}",
                    param.name,
                    operation
                        .path_params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
            path = path.replace(&format!("{{{}}}", param.name), &wert);
        }

        let query: Vec<(String, String)> = query
            .unwrap_or_default()
            .into_iter()
            .map(|(name, wert)| (name, as_text(&wert)))
            .collect();

        // **Der Aufruf selbst blockiert, und das muss aus dem Faden.**
        // reqwest::blocking legt sich eine eigene Laufzeit an und
        // bricht ab, wenn es aus einem Tokio-Arbeiter gerufen wird. Der
        // ganze Rest hier ist async, weil das SDK es ist.
        let upload = if operation.wants_file {
            let bytes = file_bytes(file.as_deref(), file_base64.as_deref())?;
            let content_type = l3pus_cli::api::file_content_type(&bytes, &operation.file_type);
            Some((bytes, content_type))
        } else {
            None
        };

        let server = self.server.clone();
        let token = self.token.clone();
        let method = operation.method.clone();
        let response = tokio::task::spawn_blocking(move || {
            let client = Client::new(&server, token)?;
            match upload {
                Some((bytes, content_type)) => {
                    client.call_with_file(&method, &path, &query, bytes, &content_type)
                }
                None => client.call(&method, &path, &query, body.as_ref()),
            }
        })
        .await
        .map_err(|error| {
            l3pus_cli::Failure::unreachable(format!("Aufruf abgebrochen: {error}"))
        })??;

        answer(&response, &group, &command)
    }
}

/// What a model gets back for a server's answer.
///
/// **An image goes back as an image.** A page preview exists so that an
/// agent can look at what it built; handed a file path instead, a model in
/// a desktop app sees a string and nothing else. The image travels with a
/// line of text that says which page it is and how many there are.
///
/// **Other bytes land in a file, not in the answer.** Ein PDF als Text
/// wäre ein halbes Megabyte Unrat im Kontext und danach immer noch kein
/// PDF. Der Pfad dagegen ist etwas, das ein Mensch öffnen und ein Agent
/// weiterreichen kann.
fn answer(
    response: &l3pus_cli::api::Response,
    group: &str,
    command: &str,
) -> Outcome<CallToolResult> {
    match l3pus_cli::exit::from_status(response.status) {
        Code::Ok if is_image(&response.content_type) => {
            use base64::Engine;
            let mime = response.content_type.split(';').next().unwrap_or("").trim();
            let encoded = base64::engine::general_purpose::STANDARD.encode(&response.bytes);
            let mut note = serde_json::json!({
                "ok": true,
                "status": response.status,
                "content_type": mime,
            });
            if let Some(pages) = response.header("x-page-count") {
                note["page"] = serde_json::json!(response.header("x-page").unwrap_or("1"));
                note["pages"] = serde_json::json!(pages);
            }
            Ok(CallToolResult::success(vec![
                ContentBlock::text(pretty(&note)),
                ContentBlock::image(encoded, mime),
            ]))
        }
        Code::Ok if !response.is_json() => {
            let path = write_to_file(&response.bytes, &response.content_type, group, command)?;
            Ok(CallToolResult::success(vec![ContentBlock::text(pretty(
                &serde_json::json!({
                    "ok": true,
                    "status": response.status,
                    "content_type": response.content_type,
                    "bytes": response.bytes.len(),
                    "file": path,
                    "note": "Die Antwort ist kein Text und liegt als Datei. Der Pfad oben zeigt darauf.",
                }),
            ))]))
        }
        Code::Ok => Ok(CallToolResult::success(vec![ContentBlock::text(pretty(
            &serde_json::json!({
                "ok": true,
                "status": response.status,
                "result": response.body,
            }),
        ))])),
        code => Err(l3pus_cli::Failure {
            code,
            message: l3pus_cli::api::complain(response.status, &response.body),
        }),
    }
}

/// Raster images a model can look at. SVG is not among them: models take
/// PNG, JPEG, GIF and WebP, and an SVG of a page draws its text as paths.
fn is_image(content_type: &str) -> bool {
    matches!(
        content_type.split(';').next().unwrap_or("").trim(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// The bytes of a file to upload, from a path or from Base64.
fn file_bytes(path: Option<&str>, encoded: Option<&str>) -> Outcome<Vec<u8>> {
    match (path, encoded) {
        (Some(path), _) => std::fs::read(path)
            .map_err(|error| l3pus_cli::Failure::usage(format!("{path}: {error}"))),
        (None, Some(encoded)) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|error| {
                    l3pus_cli::Failure::usage(format!("file_base64 ist kein Base64: {error}"))
                })
        }
        (None, None) => Err(l3pus_cli::Failure::usage(
            "Dieser Befehl will eine Datei: file (ein Pfad) oder file_base64.",
        )),
    }
}

#[tool_handler]
impl ServerHandler for Rakete {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(format!(
            "Rakete ist das Betriebssystem eines Handwerks- oder Dienstleistungsbetriebs: \
                 Kunden, Tickets, Plantafel, Zeiten, Belege, Verträge, Lager, Außendienst.\n\n\
                 Dieser Server spricht mit {}. Er hat {} Befehle in {} Bereichen, und sie kommen \
                 aus der Beschreibung dieses Servers, nicht aus einer Liste hier.\n\n\
                 Vorgehen: rakete_groups für den Überblick, rakete_search wenn klar ist, was \
                 gebraucht wird, rakete_operations für die Befehle eines Bereichs, \
                 rakete_describe für einen Befehl in allen Einzelheiten, rakete_call zum \
                 Ausführen. Vor jedem schreibenden Aufruf rakete_describe lesen: dort steht \
                 der Ablauf (was vorher da sein muss, was danach kommt) und jedes Feld mit \
                 seinen erlaubten Werten.\n\n\
                 Manche Befehle liefern ein Bild (returns: image/png), etwa die Vorschau einer \
                 Seite oder einer Gestaltung. Es kommt als Bild zurück: ansehen, bevor etwas \
                 abgeschlossen oder gespeichert wird. rakete_search mit \"Bild\" oder \
                 \"Vorschau\" findet sie.\n\n\
                 Lesen ist unbedenklich. Alles Schreibende ändert echte Daten in einem laufenden \
                 Betrieb: eine Rechnung, ein Einsatz, eine Arbeitszeit. Vorher fragen.",
            self.server,
            self.catalog().count(),
            self.catalog().groups.len()
        ));
        info
    }
}

/// Startet den Server auf stdin und stdout.
///
/// **Nichts darf sonst nach stdout.** Dort läuft das Protokoll, und eine
/// einzige Zeile Begleittext macht die Verbindung kaputt, ohne dass der
/// Benutzer je erfährt, warum. Deshalb geht jeder Hinweis nach stderr.
pub fn serve(catalog: Arc<Catalog>, server: String, token: Option<String>) -> Outcome<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            l3pus_cli::Failure::unreachable(format!("Keine Laufzeitumgebung: {error}"))
        })?;

    runtime.block_on(async move {
        let service = Rakete::new(catalog, server, token)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| {
                l3pus_cli::Failure::unreachable(format!("MCP kam nicht zustande: {error}"))
            })?;
        service.waiting().await.map_err(|error| {
            l3pus_cli::Failure::unreachable(format!("MCP abgebrochen: {error}"))
        })?;
        Ok(())
    })
}

/// Eine Antwort, die kein Text ist, in eine Datei legen.
///
/// **Im Ordner für Flüchtiges und nicht im Arbeitsverzeichnis.** Ein
/// Werkzeug, das ungefragt Dateien neben den Quelltext legt, tut etwas,
/// das niemand verlangt hat. Der Name trägt Bereich, Befehl und die Zeit,
/// damit zwei Abrufe sich nicht überschreiben.
fn write_to_file(bytes: &[u8], content_type: &str, group: &str, command: &str) -> Outcome<String> {
    let extension = match content_type.split(';').next().unwrap_or("").trim() {
        "application/pdf" => "pdf",
        "application/xml" | "text/xml" => "xml",
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "text/csv" => "csv",
        "application/zip" => "zip",
        _ => "bin",
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|dauer| dauer.as_millis())
        .unwrap_or(0);
    let directory = std::env::temp_dir().join("rakete");
    std::fs::create_dir_all(&directory).map_err(|error| {
        l3pus_cli::Failure::unreachable(format!(
            "{} ließ sich nicht anlegen: {error}",
            directory.display()
        ))
    })?;
    let path = directory.join(format!("{group}-{command}-{stamp}.{extension}"));
    std::fs::write(&path, bytes)
        .map_err(|error| l3pus_cli::Failure::unreachable(format!("{}: {error}", path.display())))?;
    Ok(path.display().to_string())
}

/// Ein Wert als Text, ohne Anführungszeichen um Zeichenketten.
///
/// Ein Modell schreibt `{"id": 42}` genauso oft wie `{"id": "42"}`, und
/// beides meint dieselbe Zeile in der Datenbank.
fn as_text(wert: &serde_json::Value) -> String {
    match wert {
        serde_json::Value::String(text) => text.clone(),
        andere => andere.to_string(),
    }
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Damit `ErrorData` nicht ungenutzt gemeldet wird, falls das SDK seine
/// Signaturen ändert: der Typ gehört zur Schnittstelle, die wir bedienen.
#[allow(dead_code)]
type UnusedErrorData = ErrorData;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;

    fn katalog() -> Arc<Catalog> {
        let document = serde_json::json!({
            "paths": {
                "/api/customers/{id}": {
                    "get": {"tags": ["http::customers"], "operationId": "get_customer",
                            "summary": "Ein einzelner Kunde",
                            "parameters": [{"name": "id", "in": "path", "required": true}]}
                },
                "/api/customers": {
                    "get": {"tags": ["http::customers"], "operationId": "list_customers",
                            "summary": "Die Kunden dieses Betriebs"},
                    "post": {"tags": ["http::customers"], "operationId": "create_customer",
                             "summary": "Einen Kunden anlegen",
                             "requestBody": {"content": {"application/json":
                                 {"schema": {"required": ["type", "last_name"]}}}}}
                },
                "/api/dispatch/board": {
                    "get": {"tags": ["http::dispatch"], "operationId": "get_board",
                            "summary": "Die Plantafel für einen Zeitraum"}
                }
            }
        });
        Arc::new(spec::parse(&document).unwrap())
    }

    fn server() -> Rakete {
        Rakete::new(katalog(), "http://127.0.0.1:3000".into(), None)
    }

    #[test]
    fn the_overview_names_every_group_with_a_count() {
        let response: serde_json::Value = serde_json::from_str(&server().rakete_groups()).unwrap();
        let gruppen = response["groups"].as_array().unwrap();
        assert_eq!(gruppen.len(), 2);
        let namen: Vec<_> = gruppen
            .iter()
            .map(|g| g["group"].as_str().unwrap())
            .collect();
        assert!(namen.contains(&"customers"), "{namen:?}");
        assert!(namen.contains(&"dispatch"), "{namen:?}");
    }

    #[test]
    fn searching_finds_across_groups_and_in_german() {
        // Gesucht wird in der Beschreibung, und die ist deutsch. Ein
        // Modell, das "Plantafel" liest, soll danach suchen können.
        let hits: serde_json::Value =
            serde_json::from_str(&server().rakete_search(Parameters(SearchParams {
                term: "Plantafel".into(),
            })))
            .unwrap();
        assert_eq!(hits["found"], 1);
        assert_eq!(hits["matches"][0]["group"], "dispatch");
    }

    #[test]
    fn searching_is_case_insensitive() {
        let hits: serde_json::Value =
            serde_json::from_str(&server().rakete_search(Parameters(SearchParams {
                term: "KUNDEN".into(),
            })))
            .unwrap();
        assert!(hits["found"].as_u64().unwrap() >= 2, "{hits}");
    }

    #[test]
    fn the_details_carry_what_a_call_needs() {
        let response: serde_json::Value =
            serde_json::from_str(&server().rakete_operations(Parameters(GroupParams {
                group: "customers".into(),
            })))
            .unwrap();
        let befehle = response["commands"].as_array().unwrap();
        let get = befehle.iter().find(|c| c["command"] == "get").unwrap();
        assert_eq!(get["path_values"][0], "id");
        let create = befehle.iter().find(|c| c["command"] == "create").unwrap();
        assert_eq!(create["body"]["required_fields"][0], "type");
    }

    #[test]
    fn an_unknown_group_answers_with_the_ones_that_exist() {
        // Sonst rät ein Modell weiter statt nachzusehen.
        let response: serde_json::Value =
            serde_json::from_str(&server().rakete_operations(Parameters(GroupParams {
                group: "kunden".into(),
            })))
            .unwrap();
        assert!(response["error"].is_string());
        assert!(response["groups"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn describing_a_command_gives_the_whole_body_and_the_flow() {
        let document = serde_json::json!({
            "paths": {"/api/themes/preview": {"post": {
                "tags": ["http::themes"], "operationId": "preview",
                "summary": "Einen Satz als Bild ansehen, ohne ihn zu speichern.",
                "description": "Danach add-version zum Speichern.",
                "requestBody": {"content": {"application/json":
                    {"schema": {"$ref": "#/components/schemas/PreviewRequest"}}}},
                "responses": {"200": {"description": "PNG", "content": {"image/png": {}}}}
            }}},
            "components": {"schemas": {"PreviewRequest": {"type": "object", "required": ["spec"],
                "properties": {"spec": {"type": "object", "properties": {
                    "stil": {"type": "string", "enum": ["haarlinien", "zebra"]}}}}}}}
        });
        let server = Rakete::new(
            Arc::new(spec::parse(&document).unwrap()),
            "http://127.0.0.1:3000".into(),
            None,
        );
        let detail: serde_json::Value =
            serde_json::from_str(&server.rakete_describe(Parameters(DescribeParams {
                group: "themes".into(),
                command: "preview".into(),
            })))
            .unwrap();
        assert!(
            detail["description"]
                .as_str()
                .unwrap()
                .contains("add-version"),
            "{detail}"
        );
        assert_eq!(
            detail["body"]["schema"]["properties"]["spec"]["properties"]["stil"]["enum"][1],
            "zebra"
        );
        assert_eq!(detail["returns"][0], "image/png");
    }

    #[test]
    fn describing_an_unknown_command_names_the_ones_that_exist() {
        let detail: serde_json::Value =
            serde_json::from_str(&server().rakete_describe(Parameters(DescribeParams {
                group: "customers".into(),
                command: "anlegen".into(),
            })))
            .unwrap();
        assert!(detail["error"].is_string());
        assert!(
            detail["commands"].as_array().unwrap().len() >= 2,
            "{detail}"
        );
    }

    fn response(
        content_type: &str,
        bytes: &[u8],
        headers: &[(&str, &str)],
    ) -> l3pus_cli::api::Response {
        l3pus_cli::api::Response {
            status: 200,
            body: serde_json::Value::Null,
            bytes: bytes.to_vec(),
            content_type: content_type.into(),
            headers: headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn an_image_comes_back_as_an_image_with_its_page() {
        // The whole point of a page preview: the model sees the page, not
        // a path to a file it cannot open.
        let result = answer(
            &response(
                "image/png",
                b"\x89PNG-bytes",
                &[("x-page-count", "3"), ("x-page", "2")],
            ),
            "documents",
            "page-image",
        )
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        let blocks = value["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "image", "{value}");
        assert_eq!(blocks[1]["mimeType"], "image/png");
        let note: serde_json::Value =
            serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(note["pages"], "3");
        assert_eq!(note["page"], "2");
    }

    #[test]
    fn an_svg_is_not_handed_over_as_an_image() {
        // Models take raster images; an SVG goes to a file like a PDF.
        assert!(!is_image("image/svg+xml"));
        assert!(is_image("image/png; charset=binary"));
    }

    #[test]
    fn a_file_comes_from_a_path_or_from_base64_and_otherwise_is_asked_for() {
        assert_eq!(file_bytes(None, Some("aGFsbG8=")).unwrap(), b"hallo");
        assert!(file_bytes(None, Some("kein base64!")).is_err());
        assert!(file_bytes(None, None).is_err());
    }

    #[test]
    fn a_number_in_the_path_is_the_same_as_a_string() {
        assert_eq!(as_text(&serde_json::json!(42)), "42");
        assert_eq!(as_text(&serde_json::json!("42")), "42");
    }

    #[test]
    fn the_instructions_say_that_writing_touches_a_real_business() {
        // Der eine Satz, der verhindert, dass ein Modell munter eine
        // Rechnung abschließt, weil es gerade danach gefragt wurde.
        let info = server().get_info();
        let text = info.instructions.unwrap();
        assert!(text.contains("Vorher fragen"), "{text}");
        assert!(text.contains("echte Daten"), "{text}");
    }
}
