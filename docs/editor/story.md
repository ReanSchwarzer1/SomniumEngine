# Story: dialogue and quests

Dialogue and quests are written on the graph surface. Find **Dialogue Graph** or **Quest Graph** in the command palette, or use the Dialogue and Quest buttons on the graph toolbar. **Open** picks a `.somdialogue` or `.somquest` file from the project's `story` folder, and **Save** writes it back. The file is the source of truth. The graph is a view of it and keeps everything it cannot draw, such as text keys and plural forms.

The sample in `assets/story` is a lamplighter and a quest to relight the street's lamps. Open it to see each kind of node.

## Dialogue nodes

- **Start / Entry**: where the dialogue begins. An Entry is a named way in that other dialogues and the game can call.
- **Line**: a speaker says something. It takes an expression for the portrait, voice, tags, a When condition and effects.
- **Option**: a choice the player can take. It can have a condition, a skill or language check, run once only, or have a time limit. A failed roll goes to its Fail wire.
- **Branch**: goes down the first wire whose condition holds.
- **Action**: runs effects. **Call**: plays another dialogue and comes back. **Jump**: goes to another dialogue's node. **End**: the conversation ends with a result.
- The order of the wires is the drawing order, top to bottom. That is the order choices are offered in.
- **Go To** nodes are made for you where a line loops back or leads into another dialogue. They are not saved as nodes.

Select a node to edit it in the inspector on the right. Long text has room there, and Shift+Enter starts a new line. The Id field renames the node, and its wires follow the new name.

## Conditions and effects

- A condition is one line: `met.mira and rep.guild >= 2`, `visited('n4')`, `quest('lamps') == 'active'`.
- Effects are separated by semicolons: `gold -= 5; met.mira = true; start_quest('lamps')`.
- A gate check is written `Patter: lang.patter >= 'fluent'`. A roll is written `Athletics: skill.athletics + 2 + d20 >= 12`, and a penalty as `- 3`.
- Facts and their types come from the project's `.somstory` manifest. A misspelt fact or a value of the wrong type is an error at the character where it happens.

## Check, Find and Play

- **Check** tests the open graph against the whole project: speakers, expressions, voices, the quests and stages it names, and what each command expects. Problems outline their nodes in red (errors) or amber (warnings). Hover over a node to read the problem. The next edit clears the marks.
- **Find** searches ids, text and notes. Arrows move through the results, Enter selects and shows the node, and Esc closes the search.
- **Play** plays the dialogue from the selected node, or from the start when nothing is selected. The panel shows each line, the choices and every fact that changes. Keys 1 to 9 choose, Space goes on and Esc stops.
- The **setup** field runs effects before playing, which is how you test one branch: `lang.patter = 'fluent'`.
- **Game View** shows the play-through in the game's dialogue presenter, with portraits, typewriter text and quest notices. Click or press Space to go on. Esc leaves the view.

## Quests

- **Quest**: the quest's title, summary, giver, when it starts and when it fails. Its wire leads to the first stage.
- **Stage**: journal lines, objectives, effects on entering, and up to four **Next** wires, each with a When. A stage with an End completes or fails the quest.
- **Objective**: done or failed when a condition holds, or a counter (`fact >= target`) or a timer. It can be optional or hidden.
- An objective's marker is an anchor name, or `(x, y, z)` in game metres. Add `region=<name>` to measure from that region's anchor, then `r=<metres>` and `arrive` to complete it when the player gets there: `strongroom_door r=2 arrive`.
- Prefer anchors over positions. An anchor follows its place if the place moves.
- Markers and radii are in game metres, but the text the player reads keeps the story's distances ("three days' ride").

## Story actors

Add **Story Actor** to a character and set **Speaker** to its id in the manifest's cast. During a scene the game updates the read-only Performance fields: talking, jaw, lids, expression, look, gesture and mark. When **Drive Look** is on, a rig's Look At turns to whoever the scene says. Scripts can read the Performance fields.

## Tools and automation

The `somnium_story` MCP tool, or an authoring query of kind `story`, gives an overview of the story content. It can also list the schema, lint, check, simulate a dialogue with scripted choices, export strings for translation, and search.
