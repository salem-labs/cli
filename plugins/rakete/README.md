# Rakete für Claude Code und Claude Desktop

Fragt den Rakete-Server eines Betriebs: Kunden, Einsätze, Zeiten, Belege,
Verträge, Lager. **Alle Befehle des Servers**, hinter fünf Werkzeugen.

## Einrichten

Zwei Schritte, und der erste ist derselbe wie für die Kommandozeile.

```bash
# 1. Das Werkzeug installieren und einmal anmelden
cargo install --git https://github.com/l3pus-labs/cli rakete-cli
rakete login --server https://rakete-app.example.de

# 2. Das Plugin installieren
/plugin marketplace add l3pus-labs/cli
/plugin install rakete@l3pus
```

**Keine Umgebungsvariablen nötig.** `rakete login` legt das Zugangszeichen
im Schlüsselbund des Systems ab und merkt sich die Adresse des Servers;
das Plugin nimmt beides von dort.

Für ein Programm, das sich nicht anmelden kann, gehen auch `RAKETE_SERVER`
und `RAKETE_TOKEN` als Umgebungsvariablen. Ein Zeichen legt man in Rakete
unter *Einstellungen, Zugang, Zeichen für Maschinen* an; es darf entweder
nur lesen oder auch schreiben, gilt höchstens neunzig Tage und lässt sich
jederzeit zurückziehen.

## Die fünf Werkzeuge

| Werkzeug | Wofür |
|---|---|
| `rakete_groups` | Welche Bereiche es gibt, mit der Anzahl ihrer Befehle |
| `rakete_search` | Einen Befehl über alle Bereiche hinweg finden |
| `rakete_operations` | Die Befehle eines Bereichs, in Kurzform |
| `rakete_describe` | Ein Befehl in allen Einzelheiten: Ablauf, jedes Feld mit Typ und erlaubten Werten, was zurückkommt |
| `rakete_call` | Einen Befehl ausführen. Bilder kommen als Bild zurück, eine Datei zum Hochladen geht als Pfad oder Base64 hinaus |

**Fünf und nicht dreihundert, und das ist Absicht.** Rakete hat rund
dreihundert Vorgänge. Als einzelne Werkzeuge nebeneinander würden sie den
Kontext füllen und die Auswahl verschlechtern: ein Modell, das aus
dreihundert ähnlich klingenden Namen wählt, wählt schlechter als eines,
das erst sucht und dann aufruft. **Erreichbar ist trotzdem alles**, denn
`rakete_call` führt jeden dieser Vorgänge aus.

Die Befehle kommen aus der Beschreibung, die der Server selbst ausliefert.
Ein neuer Endpunkt in Rakete ist hier sofort da, ohne dass jemand dieses
Plugin anfasst, und ein Betrieb mit einer älteren Installation bekommt
genau die Befehle, die sein Server kennt.

## Was es ändern kann

Ein Zeichen mit Schreibrecht kann alles, was der Mensch darf, in dessen
Namen es spricht: eine Rechnung schreiben, einen Einsatz verschieben, eine
Arbeitszeit eintragen. **Das sind echte Daten in einem laufenden
Betrieb.** Der Server sagt Claude das beim Verbinden, und ein Zeichen mit
`rakete_read_` am Anfang weist jeden schreibenden Aufruf ab.

Wer nur Auskunft will, nimmt ein lesendes Zeichen. Das ist die Vorgabe.

## Antworten, die keine Daten sind

Ein PDF, eine E-Rechnung, ein Foto: solche Antworten landen als Datei im
Ordner für Flüchtiges, und das Werkzeug gibt den Pfad zurück. Ein halbes
Megabyte PDF im Kontext wäre kein PDF, sondern Unrat.
