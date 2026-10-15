#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';

const decode = name => `engine::decode::tests::${name}`;
const project = name => `engine::project::tests::${name}`;
const cases = {
  'audio-tone-codecs': {
    appearance:[decode('pcm_eof_preserves_exact_duration_channels_and_samples'),decode('real_compressed_formats_decode_content_and_report_length_evidence')],
    failure:[decode('flac_truncation_and_checksum_damage_are_explicit_failures'),decode('truncated_pcm_never_returns_a_decoded_prefix_as_success'),decode('deck_decode_refuses_declared_pcm_over_its_budget_before_decoding_or_allocating_samples')],
    preservation:[],
  },
  'audio-native-session': {
    editability:[project('full_state_roundtrip_preserves_every_attachment_rack_note_and_control')],
    undo:['engine::undo::tests::renderer_controls_undo_and_redo_without_stopping_unrelated_live_owners'],
    save_reopen:[project('channel_effects_native_archive_reopen_rate_change_and_legacy_migration_preserve_both_decks'),project('fresh_process_reopens_native_project_and_matches_reference_render')],
    failure:[project('version_one_projects_migrate_empty_cue_names_and_colors_and_unknown_versions_fail')],
    preservation:[],
  },
  'audio-groove-members': {
    appearance:['engine::beatgrid::variable_tests::recorded_human_drum_pcm_survives_sync_cross_anchor_loops_and_seeks'],
    preservation:[],
  },
};
if (process.argv[2] === '--plan') {
  console.log(JSON.stringify({product:'omatainer',tier:'headless',cases,boundary:'offline software assertions; no sound device, listening or controller proof'}));
} else {
  const request = JSON.parse(fs.readFileSync(process.argv[2],'utf8'));
  const plan = cases[request.fixture.id];
  if (!plan || request.driver !== 'libtest' || !request.binary) throw Error('Known case and actual Omatainer test executable required');
  const before = crypto.createHash('sha256').update(fs.readFileSync(request.input)).digest('hex');
  const results = {};
  for (const name of [...new Set(Object.values(plan).flat())]) {
    const run = spawnSync(request.binary,['--exact',name,'--nocapture','--test-threads=1'],{encoding:'utf8',timeout:25000,maxBuffer:65536,env:process.env});
    const output = `${run.stdout ?? ''}${run.stderr ?? ''}`;
    const log = `test-${Object.keys(results).length}.log`;
    fs.writeFileSync(path.join(request.output,log),output);
    results[name] = {passed:run.status===0 && !run.error && /test result: ok\. 1 passed; 0 failed; 0 ignored;/.test(output),artifact:log};
    if (!results[name].passed) break;
  }
  const unchanged = before===crypto.createHash('sha256').update(fs.readFileSync(request.input)).digest('hex');
  fs.writeFileSync(path.join(request.output,'assertions.json'),JSON.stringify({fixture:request.fixture.id,tests:results,input_unchanged:unchanged,boundary:'Existing production decoder/project/history assertions; headless test executable, no installed app/device/physical/human evidence'},null,2));
  const checks = Object.fromEntries(Object.entries(plan).map(([key,names])=>[key,{status:(key==='preservation'?unchanged:names.every(n=>results[n]?.passed))?'passed':'failed',artifacts:['assertions.json',...names.flatMap(n=>results[n]?[results[n].artifact]:[])]}]));
  fs.writeFileSync(path.join(request.output,'result.json'),JSON.stringify({checks}));
  process.exitCode=Object.values(checks).every(c=>c.status==='passed')?0:1;
}
