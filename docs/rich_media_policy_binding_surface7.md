# Rich-media inventory and enterprise policy

annotation/media redaction modes are `inventory_only`, `preserve_inert`, `remove_active_content`, `remove_all_media`, `flatten_static_poster`, and `custom`. Reports state what remained, what was removed or flattened, whether payloads/URLs/annotations/AP remain, signature impact, rescan status, and residual risk.

Inventory covers RichMediaContent/Settings, assets/configurations/instances, embedded streams, SWF/MIME/hash/bytes, Sound/Movie/Screen, Rendition/MediaClip, 3D streams/views, external URLs, JavaScript associations, activation/deactivation data, OCG-related dictionaries, and static AP posters. Inventory never decodes a payload, launches a player, performs network/filesystem I/O, or claims playback.

`flatten_static_poster` selectively paints the valid static AP into page content, removes the media annotation and active/payload objects, and rescans. Unrelated annotations and Widgets remain.

```text
wellfriendpdf rich-media-report input.pdf
wellfriendpdf rich-media-sanitize input.pdf --policy remove_all_media --output clean.pdf --json
wellfriendpdf rich-media-flatten-poster input.pdf --output poster.pdf --json
```

If no valid static AP exists, poster flatten reports the exact limitation and removes active media under the selected policy; it does not decode video/audio/3D data to manufacture a poster.
