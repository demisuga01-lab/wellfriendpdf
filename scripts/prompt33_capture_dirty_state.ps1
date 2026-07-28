param(
    [string]$OutputDirectory = "target/prompt33-geometric-semantic-reflow"
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$baseline = '7b33a77e6da8321644734051afeaeaec59a196bc'
$authorizedStartingSnapshot = '3d9cb8fecae3edc9eab2dfd03889fce29d22ced5c161b86dd079cda8d42b08a1'

function Get-Prompt33Classification([string]$Path) {
    if ($Path -like 'bindings/*') { return 'intended_prompt33_binding' }
    if ($Path -like 'docs/*') { return 'intended_prompt33_doc' }
    if ($Path -like 'scripts/*') { return 'intended_prompt33_script' }
    if ($Path -like 'fuzz/*' -or $Path -like '*tests/*') { return 'intended_prompt33_test' }
    if ($Path -eq 'Cargo.lock' -or $Path -like 'crates/*') { return 'intended_prompt33_source' }
    return 'uncertain'
}

Push-Location $repository
try {
    $tracked = @(git diff --name-only)
    $untracked = @(git ls-files --others --exclude-standard)
    $entries = foreach ($path in @($tracked + $untracked | Sort-Object -Unique)) {
        $item = Get-Item -LiteralPath (Join-Path $repository $path)
        [ordered]@{
            path = $path
            kind = if ($tracked -contains $path) { 'tracked_modified' } else { 'untracked' }
            classification = Get-Prompt33Classification $path
            bytes = $item.Length
            sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $canonical = ($entries | Sort-Object path | ForEach-Object {
        "$($_.kind)|$($_.path)|$($_.bytes)|$($_.sha256)|$($_.classification)"
    }) -join "`n"
    $hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        $snapshot = ([BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($canonical)))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
    $now = [DateTime]::UtcNow.ToString('o')
    $head = (git rev-parse HEAD).Trim()
    $originMain = (git rev-parse origin/main).Trim()
    $common = [ordered]@{
        captured_at_utc = $now
        baseline = $baseline
        authorized_dirty_snapshot_at_prompt33c_start = $authorizedStartingSnapshot
        current_head = $head
        origin_main = $originMain
        branch = (git branch --show-current).Trim()
        remote_origin = (git remote get-url origin).Trim()
        deterministic_manifest_sha256 = $snapshot
        entries = @($entries | Sort-Object path)
    }
    $state = [ordered]@{
        schema_version = 'prompt33.authorized-dirty-state.v9'
        status = 'authorized_continuation_from_existing_dirty_prompt33_worktree'
        original_clean_prompt32_baseline = $baseline
        authorized_dirty_snapshot_at_prompt33c_start = $authorizedStartingSnapshot
        changed_path_count = @($entries).Count
    }
    foreach ($property in $common.GetEnumerator()) { $state[$property.Key] = $property.Value }
    $manifest = [ordered]@{ schema_version = 'prompt33.worktree-manifest.v9' }
    foreach ($property in $common.GetEnumerator()) { $manifest[$property.Key] = $property.Value }
    $gaps = [ordered]@{
        schema_version = 'prompt33.remaining-gap-matrix.v10'
        captured_at_utc = $now
        baseline = $baseline
        current_dirty_snapshot_sha256 = $snapshot
        closure_status = 'candidate_complete_pending_single_closure_commit'
        implemented_boundaries = @(
            'source_linked_geometric_single_region_rewrite',
            'bounded_changed_length_grapheme_safe_multi_run_source_style_rewrite_with_existing_font_cmaps',
            'single_text_state_only_mcid_bdc_relocation_without_duplicate_mcid_ownership',
            'uax14_grapheme_safe_break_candidates_with_final_shaped_metrics',
            'audited_en_us_and_es_dictionary_hyphenation_with_logical_extraction_preserving_visual_hyphen_output',
            'bounded_dynamic_final_layout',
            'cassowary_single_region_feasibility',
            'explicit_allowed_region_expansion_source_rewrite',
            'prompt06_xycut_semantic_runtime_region_graph',
            'bounded_runtime_column_nodes_with_contains_and_next_column_edges',
            'semantic_document_bounded_document_scope_with_geometric_page_local_invalidation',
            'deterministic_precedence_cycle_resolution',
            'incremental_reflow_mutation_session_undo',
            'single_paragraph_same_page_explicit_next_region_flow_with_positioned_canonical_source_rewrite',
            'single_paragraph_same_page_explicit_ltr_next_column_flow_with_positioned_canonical_source_rewrite',
            'single_paragraph_same_page_explicit_rtl_next_column_flow_with_actualtext_logical_extraction',
            'single_paragraph_existing_next_page_flow_into_semantically_proven_empty_region',
            'single_paragraph_catalog_preserving_explicit_page_creation',
            'bounded_explicit_dependency_linked_same_page_path_movement_with_atomic_preimage_undo',
            'bounded_explicit_source_link_annotation_rect_and_quadpoint_movement_with_atomic_preimage_undo',
            'direct_geometric_unaffected_page_stream_and_extraction_proof',
            'cross_binding_canonical_overflow_constraint_and_confidence_query_reports',
            'cross_binding_reflow_output_validation',
            'cross_binding_replay_verified_executable_undo_reflow'
        )
        documented_typed_limits = @(
            'arbitrary_nested_or_partial_tag_rewrite',
            'unproven_generic_object_dependency_movement',
            'inferred_page_creation_without_explicit_policy_and_source_flow',
            'reference_repair_without_exact_source_association',
            'unsupported_vertical_or_script_specific_source_serialization'
        )
        blocking_gates = @()
        release = @{
            closure_commit_permitted = $true
            evidence_requirement = 'retained_vps_stage_exit_artifacts_and_hashes'
            post_commit_requirements = @('origin_main_push', 'fetch_and_head_equality', 'clean_worktree')
        }
    }
    $target = Join-Path $repository $OutputDirectory
    New-Item -ItemType Directory -Force -Path $target | Out-Null
    $utf8 = [Text.UTF8Encoding]::new($false)
    foreach ($record in @(
        @('prompt33-authorized-dirty-state.json', $state),
        @('prompt33-worktree-manifest.json', $manifest),
        @('prompt33-remaining-gap-matrix.json', $gaps)
    )) {
        [IO.File]::WriteAllText(
            (Join-Path $target $record[0]),
            ($record[1] | ConvertTo-Json -Depth 12) + [Environment]::NewLine,
            $utf8
        )
    }
    Write-Output "SNAPSHOT=$snapshot PATHS=$(@($entries).Count)"
}
finally {
    Pop-Location
}
