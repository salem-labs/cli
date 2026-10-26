# l3pus cli

**Ein Binär für zwei Leser**: einen Menschen am Terminal und ein Programm
über `--json`.

Hier liegen die Kommandozeilenwerkzeuge zu den Produkten von l3pus. Das
erste ist `rakete`, und es kann **alles, was der Server kann**, weil es
seine Befehle nicht mitbringt, sondern sich beim Server holt.

```bash
rakete customers list
rakete customers get 42 --json
rakete dispatch board --from 2026-09-14 --to 2026-09-28 --json
rakete customers create -f type=business -f company_name="Muster GmbH" -f last_name=Muster
```

---

## Warum es das gibt

Rakete ist ein Betriebssystem für Handwerk und technische Dienstleistung.
Wer es benutzt, sitzt normalerweise davor. Zwei Gruppen sitzen nicht
davor:

- **Der Betrieb selbst**, der etwas automatisieren will: nachts einen
  Bericht ziehen, morgens die offenen Posten in eine Tabelle schreiben,
  einen Stapel Kunden aus dem alten Programm übernehmen.
- **Eine KI**, die für den Betrieb arbeitet. Und zwar jede, nicht eine
  bestimmte: wer Claude benutzt, wer ChatGPT benutzt, wer sein eigenes
  Modell laufen lässt.

**Kein MCP, sondern eine Kommandozeile.** Ein MCP-Server bindet an einen
Anbieter und an ein Protokoll, das sich alle sechs Monate ändert. Eine
Kommandozeile kann jedes Werkzeug aufrufen, das eine Shell hat, und das
sind alle. Wer trotzdem MCP will, schreibt zwanzig Zeilen drumherum.

---

## Installation

```bash
cargo install --git https://github.com/l3pus-labs/cli rakete-cli
```

Fertige Dateien für macOS, Linux und Windows hängen an jeder
Veröffentlichung.

---

## Anmelden

**Ein Mensch** meldet sich mit Adresse und Kennwort an. Das Zeichen landet
im Schlüsselbund des Systems, nicht in einer Datei:

```bash
rakete login --server https://app.rakete.systems
```

**Eine Maschine bekommt nie ein Kennwort.** Sie bekommt ein Zeichen, das
in Rakete unter *Einstellungen, Zugang, Zeichen für Maschinen* angelegt
wird:

```bash
export RAKETE_SERVER=https://app.rakete.systems
export RAKETE_TOKEN=rakete_read_…
rakete customers list --json
```

Ein Zeichen hat einen Umfang, und die Vorgabe ist die vorsichtige:

| Anfang der Marke | Was es darf |
|---|---|
| `rakete_read_…` | lesen, sonst nichts. Jeder schreibende Aufruf wird abgewiesen |
| `rakete_write_…` | alles, was der Mensch darf, in dessen Namen es spricht |

**Man sieht es der Marke an**, und das ist der Zweck: wer in einer fremden
Konfigurationsdatei über ein `rakete_write_…` stolpert, weiß sofort, dass
das Ding Daten ändern kann.

Ein Zeichen gilt höchstens neunzig Tage, kann jederzeit zurückgezogen
werden, und **kann kein weiteres Zeichen ausstellen**. Sonst reicht ein
einziges weggekommenes für einen Zugang, der nie abläuft.

---

## Für einen Agenten

Drei Sachen, von der ersten Zeile an und nicht nachgerüstet.

### `--json` auf allem

Ergebnisse gehen nach stdout, alles andere nach stderr. `rakete … --json |
jq` bekommt nie einen Begleitsatz zu sehen. Das menschliche Format ist
das, das sich ändern darf.

### `rakete describe`

Gibt jeden Befehl, jedes Argument und jeden Typ als JSON aus, **gelesen
aus derselben Quelle wie die Befehle selbst**. Ein Agent muss nie
`--help` auseinandernehmen, und veralten kann es nicht.

```bash
rakete describe | jq '.groups.customers'
```

### Rückgabewerte, die etwas heißen

| Wert | Bedeutung |
|---|---|
| 0 | Es hat geklappt. |
| 1 | Die Sache, nach der gefragt wurde, stimmt nicht: eine Ablehnung, ein fehlender Datensatz, eine verletzte Regel. **Noch einmal ändert nichts.** |
| 2 | Der Befehl war falsch benutzt. Das liegt am Aufrufer. |
| 3 | Wir konnten nicht nachsehen: kein Zugang, kein Netz, eine Antwort, die wir nicht verstehen. **Die Frage ist noch offen.** |

**Die Trennung von 1 und 3 ist der ganze Punkt.** Ein Skript, das den
Server nicht erreicht, und eines, das ein echtes Problem gefunden hat,
geben sonst beide 1 zurück, und dann wird eine rote Kette neu gestartet
statt gelesen.

---

## Wie die Befehle entstehen

Rakete hat rund dreihundert Vorgänge. Die von Hand als Unterbefehle zu
schreiben wäre einmal viel Arbeit und danach für immer eine Liste, die
hinterherhinkt: der nächste Endpunkt käme dazu, niemand trüge ihn nach,
und das Werkzeug wäre still unvollständig.

Stattdessen liest `rakete` beim Start die Beschreibung, die der Server
selbst ausliefert (`/api/openapi.json`, offen zugänglich), und baut daraus
seinen Befehlsbaum:

```
Schildchen  http::customers     →  Gruppe    customers
Kennung     list_customers      →  Befehl    list
Pfadangabe  /api/customers/{id} →  Stellung  rakete customers get 42
Abfrage     ?from=…             →  Schalter  --from …
Rumpf       requestBody         →  --data / --field
```

**Zur Laufzeit und nicht beim Bauen**, denn zwei Betriebe können
verschiedene Fassungen von Rakete laufen haben. Jeder bekommt die
Befehle, die sein Server auch kennt. Die Beschreibung wird je Server
zwischengelagert; `rakete refresh` holt sie neu.

Daraus folgt auch: `--help`, die Vervollständigung und die Handbuchseite
kommen aus demselben Baum und können nicht auseinanderlaufen.

```bash
rakete completions zsh > ~/.zsh/completions/_rakete
rakete man > /usr/local/share/man/man1/rakete.1
```

---

## Einen Rumpf mitgeben

Drei Wege, und `--field` ist der für die Hand:

```bash
# einzelne Felder; Zahlen und ja/nein werden erkannt
rakete customers create -f type=business -f company_name="Muster GmbH" -f last_name=Muster

# fertiges JSON
rakete customers create -d '{"type":"business","last_name":"Muster"}'

# aus einer Datei, oder aus der Pipe
rakete customers create -d @kunde.json
jq -n '…' | rakete customers create -d @-
```

**Geraten wird nur, was eindeutig ist.** `menge=3` wird eine Zahl,
`aktiv=ja` wird `true`, aber `plz=028217` bleibt Text: eine Null vorn ist
Teil der Angabe und keine Ziffer zu viel.

---

## PDF, XML und Fotos

Ein Dutzend Wege in Rakete antwortet nicht mit JSON, sondern mit einem
PDF, einer E-Rechnung, einem SVG oder einem Foto. Die Bytes gehen
unverändert durch:

```bash
rakete documents document-pdf 42 -o rechnung.pdf
rakete documents document-pdf 42 > rechnung.pdf          # dasselbe
rakete documents document-einvoice 42 | xmllint --format -
```

**In ein Terminal schreibt es sie nicht.** Ein PDF auf den Bildschirm
macht ihn unbrauchbar, und die ersten Bytes eines PDF sind lesbar genug,
dass man es erst merkt, wenn es zu spät ist. Ohne Umleitung und ohne
`-o` sagt das Werkzeug stattdessen, was es hat und wie groß es ist.

---

## In Claude Desktop und Claude Code

Dasselbe Binär spricht auch das Model Context Protocol, damit Claude
Rakete bedienen kann. `rakete mcp` ist der Server; von Hand ruft man das
nicht auf.

**Claude Code:**

```bash
/plugin marketplace add l3pus-labs/cli
/plugin install rakete@l3pus
```

**Claude Desktop:** die `.mcpb`-Datei der eigenen Plattform aus der
Veröffentlichung laden und öffnen. Ein Klick, dann Adresse und Zeichen in
ein Formular, fertig.

**Fünf Werkzeuge, nicht dreihundert.** `rakete_groups` für den Überblick,
`rakete_search` zum Finden, `rakete_operations` für die Befehle eines
Bereichs, `rakete_describe` für einen Befehl in allen Einzelheiten (Ablauf,
jedes Feld mit seinen erlaubten Werten), `rakete_call` zum Ausführen.
Liefert ein Befehl ein Bild, etwa die Vorschau einer Belegseite, kommt es
als Bild beim Modell an und nicht als Pfad. Hunderte Werkzeuge
nebeneinander füllen den Kontext und verschlechtern die Auswahl; ein
Modell, das erst sucht und dann aufruft, trifft besser. **Erreichbar ist
trotzdem alles**, denn `rakete_call` führt jeden Vorgang des Servers aus.

Wer sich mit `rakete login` angemeldet hat, braucht keine
Umgebungsvariablen: der MCP-Server nimmt Adresse und Zeichen aus dem
Schlüsselbund.

---

## Woraus es besteht

| Ordner | Was |
|---|---|
| `apps/rakete` | das Binär `rakete`, Kommandozeile und MCP-Server |
| `crates/l3pus-cli` | der gemeinsame Unterbau: Anmeldung, Ausgabe, Rückgabewerte |
| `plugins/rakete` | das Plugin für Claude Code |
| `bundle` | die Beschreibung für das `.mcpb` von Claude Desktop |

Ein Arbeitsbereich und nicht ein Repository je Werkzeug: Anmeldung,
Konfiguration, Ausgabeform und Rückgabewerte sind für alle dieselben, und
drei Repositories wären drei Gelegenheiten, es unterschiedlich zu machen.

---

## Lizenz

MIT. Dieses Werkzeug spricht nur HTTP und enthält keinen Programmtext aus
Rakete selbst; deshalb kann es offen liegen, während das Produkt es nicht
tut.
