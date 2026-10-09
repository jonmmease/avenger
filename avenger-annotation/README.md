# avenger-annotation

Annotations for Avenger scenes: marks that call out data, such as leaders from labels to the points they annotate.

## Purpose

An annotation places a label away from the point that it describes and connects the two with a leader line. The leader starts at the edge of the label's box, so it depends on the label's measured size.

## Integration

- **Geometry**: `leader::leader` builds one leader from a `PlacedLabel`, which holds a label's measured bounds and placement, to a `LeaderTarget`. It needs no text engine, so code that places labels can measure each label once and test candidate leaders before it chooses them. `PlacedLabel::corners` gives the padded box that leaders start from.
- **Marks**: `leader::make_leader_marks` draws chosen leaders as path marks, where instance `i` belongs to label `i` of a text mark. `leader::make_text_leaders` measures a text mark's labels with the `LabelEngine` and draws a leader for each label that has a target.
- **Output**: a `SceneGroup` of path marks, which renderers draw and picking hits like any other path. Add the group before the labels, so that the labels draw over their leaders.
