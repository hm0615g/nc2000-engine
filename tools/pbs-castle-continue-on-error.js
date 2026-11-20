'use strict';

// Castle-only continuation helpers. This module never sends a request.
// A rejected observation must not prevent use of the CURRENT Castle form.
function attr(source, key) {
  const re = new RegExp('(?:^|\\s)' + key + '\\s*=\\s*(?:"([^"]*)"|\\x27([^\\x27]*)\\x27|([^\\s>]+))', 'i');
  const match = source.match(re);
  return match ? match[1] ?? match[2] ?? match[3] : null;
}

function classifyPageWithRecovery(response, originalClassify, readDecisionForm) {
  const page = originalClassify(response);
  if (page.kind !== 'UNKNOWN') return page;
  if (response.statusCode !== undefined &&
      !(response.statusCode >= 200 && response.statusCode < 300)) return page;
  try {
    const form = readDecisionForm(response.html || '');
    if (!['INPUT', 'CHANGE', 'BATONN'].includes(form.action)) return page;
    return {kind: 'BATTLE_' + form.action, html: response.html || '', recoveredFrom: 'RAW_FORM'};
  } catch {
    return page;
  }
}

function chooseCurrentFormFallback(rawTurnHtml, readDecisionForm) {
  try {
    const form = readDecisionForm(rawTurnHtml);
    if (!['INPUT', 'CHANGE', 'BATONN'].includes(form.action)) {
      return {status: 'WAIT_FOR_FORM', reason: 'UNSUPPORTED_ACTION'};
    }
    const selects = [...rawTurnHtml.matchAll(/<select\b([^>]*)>([\s\S]*?)<\/select\s*>/gi)]
      .filter(match => String(attr(match[1], 'name') || '').toUpperCase() === 'MOVE');
    if (selects.length !== 1) return {status: 'WAIT_FOR_FORM', reason: 'MOVE_SELECT_NOT_UNIQUE'};
    const options = [];
    for (const match of selects[0][2].matchAll(/<option\b([^>]*)>([\s\S]*?)(?=<option\b|$)/gi)) {
      if (/(?:^|\s)disabled(?:\s|=|$)/i.test(match[1])) continue;
      const value = attr(match[1], 'value');
      if (!value) continue;
      const label = match[2].replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim();
      let category = null;
      if (/^W[1-4]$/.test(value)) {
        category = 'move';
        if (/\[\s*0\s*\]/.test(label)) continue;
      } else if (/^C[1-3]$/.test(value)) {
        category = 'switch';
        if (/\[\s*0\s*\/\s*\d+\s*\]/.test(label)) continue;
      } else if (value === 'A') {
        category = 'automatic';
      }
      if (category) options.push({value, category});
    }
    if (new Set(options.map(row => row.value)).size !== options.length) {
      return {status: 'WAIT_FOR_FORM', reason: 'RAW_OPTION_DUPLICATE'};
    }
    const allowed = form.action === 'INPUT'
      ? options
      : options.filter(row => row.category === 'switch');
    const preference = form.action === 'INPUT'
      ? ['move', 'automatic', 'switch'] : ['switch'];
    const picked = preference.map(category => allowed.find(row => row.category === category))
      .find(Boolean);
    if (!picked) return {status: 'WAIT_FOR_FORM', reason: 'NO_OFFERED_PLAYABLE_OPTION'};
    return {status: 'READY', fields: {ACTION: form.action, NOW: form.now, MOVE: picked.value},
      mappingPath: 'castle-current-form-fallback', offeredOptions: options.map(row => row.value)};
  } catch (error) {
    return {status: 'WAIT_FOR_FORM', reason: 'RAW_FORM_UNREADABLE', error: String(error?.message || error)};
  }
}

function transmissionBoundary(error, guardBefore, guardAfter) {
  // Do not retry when the Castle may already have accepted a POST.
  const message = String(error?.message || error);
  if ((guardAfter?.submittedKeys?.length || 0) > (guardBefore?.submittedKeys?.length || 0)) return true;
  if (guardAfter?.halted) return true;
  return /SUBMISSION_PROGRESS_UNCONFIRMED|SUBMISSION_SESSION_HALTED|DUPLICATE_SUBMISSION_BLOCKED|SUBMISSION_ALREADY_IN_FLIGHT/.test(message);
}

module.exports = {classifyPageWithRecovery, chooseCurrentFormFallback, transmissionBoundary};