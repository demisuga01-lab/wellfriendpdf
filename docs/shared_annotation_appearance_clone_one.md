# Shared annotation appearance clone-one

For a shared annotation appearance vector, `clone_edit_one_instance` clones the edited stream and updates only the selected annotation's `/AP` entry. It supports `/N`, `/R`, `/D`, and one selected state entry such as `/N /On`, preserving sibling states and `/AS`. The source AP stream remains referenced by other annotations/widgets.

The mutation is annotation-ownership work, distinct from annotation semantics: it does not change `/AS`, field values, or sibling rollover/down appearances unless a selected category/state explicitly targets them. Malformed or ambiguous AP/state dictionaries fail closed. Structural preservation never claims cryptographic signature validity.

advanced editing closeout focused fixtures cover shared `/AP /N`, `/R`, `/D`, state
dictionaries, checkbox/radio widget state appearances, selected-state stream
cloning, `/AS` preservation, sibling-state preservation, and a shared
appearance stream whose painted vector is inside a nested Form XObject.
