//! Aus den Vorgängen einen Befehlsbaum bauen.
//!
//! Zur Laufzeit zusammengesetzt und nicht mit `#[derive(Parser)]`
//! geschrieben: die Befehle stehen in der Beschreibung des Servers und
//! nicht hier. Was clap davon bekommt, ist trotzdem ein ganz normaler
//! Baum, also gelten `--help`, die Vervollständigungen und die
//! Handbuchseite unverändert.

use clap::{Arg, ArgAction, Command};

use crate::spec::{Catalog, Operation};

/// Der ganze Baum: die eingebauten Befehle und alles aus der
/// Beschreibung.
pub fn build(catalog: Option<&Catalog>) -> Command {
    let mut root = Command::new("rakete")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Rakete von der Kommandozeile, für Menschen und für Agenten")
        .after_help(
            "Ohne Anmeldung kennt rakete nur die eingebauten Befehle.\n\
             `rakete login --server https://app.rakete.systems` holt die\n\
             Beschreibung des Servers, und danach steht hier alles, was\n\
             dieser Server kann.",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .disable_help_subcommand(true)
        .arg(
            Arg::new("server")
                .long("server")
                .short('s')
                .global(true)
                .value_name("ADRESSE")
                .help("Gegen welchen Server, statt des gemerkten"),
        )
        .arg(
            Arg::new("json")
                .long("json")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Antwort als JSON, ohne Begleittext"),
        )
        // **Ein Dutzend Wege antwortet mit einem PDF und nicht mit
        // JSON.** Ohne Ziel gehen die Bytes nach stdout, und das ist
        // richtig: `rakete documents document-pdf 12 > rechnung.pdf`
        // ist der Weg, den eine Shell ohnehin kennt. Der Schalter ist
        // für den Fall, dass jemand nebenher noch lesen will, was
        // passiert ist.
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .global(true)
                .value_name("DATEI")
                .help("Antwort in eine Datei schreiben statt nach stdout"),
        );

    root = root.subcommands(builtins());

    if let Some(catalog) = catalog {
        for (group, operations) in &catalog.groups {
            let mut sub = Command::new(group.clone())
                .about(if operations.len() == 1 {
                    "1 Vorgang".to_string()
                } else {
                    format!("{} Vorgänge", operations.len())
                })
                .subcommand_required(true)
                .arg_required_else_help(true);
            for operation in operations.values() {
                sub = sub.subcommand(command_for(operation));
            }
            root = root.subcommand(sub);
        }
    }
    root
}

/// Die Befehle, die es ohne Server gibt.
fn builtins() -> Vec<Command> {
    vec![
        Command::new("login")
            .about("Anmelden und die Beschreibung des Servers holen")
            .arg(
                Arg::new("email")
                    .long("email")
                    .value_name("ADRESSE")
                    .help("Wer sich anmeldet; sonst wird gefragt"),
            )
            .arg(
                Arg::new("token")
                    .long("token")
                    .value_name("ZEICHEN")
                    .help("Ein Zeichen für Maschinen statt Adresse und Kennwort"),
            ),
        Command::new("logout").about("Das gemerkte Zeichen vergessen"),
        Command::new("whoami").about("Wer gerade angemeldet ist"),
        Command::new("refresh").about("Die Beschreibung des Servers neu holen"),
        Command::new("describe")
            .about("Jeder Befehl mit Argumenten und Typen, als JSON")
            .long_about(
                "Gibt den ganzen Befehlsbaum als JSON aus, gelesen aus der\n\
                 Beschreibung des Servers. Damit muss ein Agent nie `--help`\n\
                 auseinandernehmen, und veralten kann es nicht.\n\n\
                 Mit Bereich und Befehl alles zu diesem einen: die ganze\n\
                 Beschreibung samt Ablauf, jedes Feld des Rumpfes mit Typ\n\
                 und erlaubten Werten, und was zurückkommt.",
            )
            .arg(Arg::new("group").value_name("BEREICH").help("Nur dieser Bereich"))
            .arg(
                Arg::new("command")
                    .value_name("BEFEHL")
                    .requires("group")
                    .help("Nur dieser Befehl, mit allen Einzelheiten"),
            ),
        Command::new("completions")
            .about("Vervollständigung für die eigene Shell")
            .arg(
                Arg::new("shell")
                    .required(true)
                    .value_parser(clap::value_parser!(clap_complete::Shell))
                    .help("bash, zsh, fish, powershell, elvish"),
            ),
        Command::new("man").about("Die Handbuchseite, nach stdout"),
        // **Für Claude Desktop und Claude Code.** Kein zweites Werkzeug,
        // sondern dasselbe Binär, das statt einer Zeile ein Protokoll
        // spricht. Wer es von Hand aufruft, sieht nichts passieren: es
        // wartet auf stdin.
        Command::new("mcp")
            .about("Als MCP-Server auf stdin und stdout sprechen")
            .long_about(
                "Spricht das Model Context Protocol, damit Claude Desktop und\n\
                 Claude Code Rakete bedienen können. Nicht von Hand aufrufen:\n\
                 das Programm wartet dann auf ein Protokoll, das niemand spricht.\n\n\
                 Fünf Werkzeuge, hinter denen alle Befehle dieses Servers stehen:\n\
                 rakete_groups, rakete_search, rakete_operations, rakete_describe,\n\
                 rakete_call.",
            ),
    ]
}

/// Ein einzelner Vorgang als Unterbefehl.
///
/// **Pfadangaben sind Stellungsargumente, alles andere sind Schalter.**
/// `rakete customers get 42` liest sich wie ein Satz; `rakete customers
/// get --id 42` wie ein Formular. Was hinten an die Adresse gehängt
/// wird, ist dagegen optional und wechselt, also Schalter.
fn command_for(operation: &Operation) -> Command {
    let about = if operation.summary.is_empty() {
        format!("{} {}", operation.method, operation.path)
    } else {
        operation.summary.clone()
    };
    let mut command = Command::new(operation.name.clone())
        .about(about)
        .long_about(format!(
            "{} {}\n\n{}",
            operation.method, operation.path, operation.description
        ));

    for param in &operation.path_params {
        command = command.arg(
            Arg::new(param.name.clone())
                .required(true)
                .value_name(param.name.to_uppercase())
                .help(if param.description.is_empty() {
                    format!("{} aus dem Pfad", param.name)
                } else {
                    param.description.clone()
                }),
        );
    }

    for param in &operation.query_params {
        // Ein Name, der mit einem Stellungsargument zusammenfällt, würde
        // clap beim Bauen umbringen. Der Zusatz ist hässlich und selten.
        let id = if operation
            .path_params
            .iter()
            .any(|other| other.name == param.name)
        {
            format!("query-{}", param.name)
        } else {
            param.name.clone()
        };
        command = command.arg(
            Arg::new(id)
                .long(param.name.clone())
                .required(param.required)
                .value_name("WERT")
                .help(if param.description.is_empty() {
                    format!("Abfragewert {}", param.name)
                } else {
                    param.description.clone()
                }),
        );
    }

    // **Eine Datei statt eines Feldsatzes.** Wo der Server Bytes will,
    // hilft `--data` nicht weiter, und `--field` erst recht nicht: die
    // Datei geht unverändert hinaus (ADR-038).
    if operation.wants_file {
        return command.arg(
            Arg::new("file")
                .long("datei")
                .value_name("PFAD")
                .required(true)
                .help("Die Datei, die hochgeladen wird. PDF oder Word"),
        );
    }

    if operation.has_body {
        let hint = if operation.required_fields.is_empty() {
            "JSON für den Rumpf, oder @datei, oder @- für stdin".to_string()
        } else {
            format!(
                "JSON für den Rumpf, oder @datei, oder @- für stdin. Nötig: {}",
                operation.required_fields.join(", ")
            )
        };
        command = command
            .arg(
                Arg::new("data")
                    .long("data")
                    .short('d')
                    .value_name("JSON")
                    .help(hint),
            )
            .arg(
                Arg::new("field")
                    .long("field")
                    .short('f')
                    .value_name("SCHLUESSEL=WERT")
                    .action(ArgAction::Append)
                    .help(
                        "Ein einzelnes Feld, mehrfach erlaubt. Zahlen und ja/nein werden erkannt",
                    ),
            );
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;

    fn catalog() -> Catalog {
        let document = serde_json::json!({
            "paths": {
                "/api/customers/{id}": {
                    "get": {"tags": ["http::customers"], "operationId": "get_customer",
                            "summary": "Ein Kunde",
                            "parameters": [{"name": "id", "in": "path", "required": true}]}
                },
                "/api/customers": {
                    "post": {"tags": ["http::customers"], "operationId": "create_customer",
                             "requestBody": {"content": {"application/json":
                                 {"schema": {"required": ["type", "last_name"]}}}}}
                }
            }
        });
        spec::parse(&document).unwrap()
    }

    #[test]
    fn without_a_server_the_builtins_are_still_there() {
        // Sonst hätte jemand ohne Anmeldung ein Werkzeug, das nicht
        // einmal sagen kann, wie man sich anmeldet.
        let command = build(None);
        let names: Vec<_> = command.get_subcommands().map(|c| c.get_name()).collect();
        assert!(names.contains(&"login"), "{names:?}");
        assert!(names.contains(&"describe"), "{names:?}");
    }

    #[test]
    fn a_path_parameter_is_a_positional() {
        let command = build(Some(&catalog()));
        let matches = command
            .try_get_matches_from(["rakete", "customers", "get", "42"])
            .expect("sollte gehen");
        let (_, sub) = matches.subcommand().unwrap();
        let (_, op) = sub.subcommand().unwrap();
        assert_eq!(op.get_one::<String>("id").unwrap(), "42");
    }

    #[test]
    fn a_missing_path_parameter_is_a_usage_error() {
        let command = build(Some(&catalog()));
        assert!(
            command
                .try_get_matches_from(["rakete", "customers", "get"])
                .is_err()
        );
    }

    #[test]
    fn the_required_fields_stand_in_the_help() {
        // Damit niemand die Beschreibung des Servers lesen muss, um
        // einen Kunden anzulegen.
        let command = build(Some(&catalog()));
        let mut create = command
            .get_subcommands()
            .find(|c| c.get_name() == "customers")
            .unwrap()
            .get_subcommands()
            .find(|c| c.get_name() == "create")
            .unwrap()
            .clone();
        let help = create.render_long_help().to_string();
        assert!(help.contains("last_name"), "{help}");
    }

    #[test]
    fn json_and_server_reach_every_subcommand() {
        let command = build(Some(&catalog()));
        let matches = command
            .try_get_matches_from(["rakete", "customers", "get", "42", "--json"])
            .expect("sollte gehen");
        assert!(matches.get_flag("json"));
    }
}
