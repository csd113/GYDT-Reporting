(() => {
    'use strict';

    const form = document.querySelector('#gydt-import-queue');
    const input = form.querySelector('#gydt-sample-zip');
    const checkInput = form.querySelector('#gydt-check-zip');
    const dropArea = form.querySelector('#gydt-zip-drop');
    const list = form.querySelector('#gydt-queue-items');
    const message = form.querySelector('#gydt-queue-message');
    const historyPanel = document.querySelector('#gydt-history');
    const historyList = historyPanel.querySelector('#gydt-history-items');
    const historyMessage = historyPanel.querySelector('#gydt-history-message');
    const refreshHistory = historyPanel.querySelector('#gydt-history-refresh');
    const clearHistory = historyPanel.querySelector('#gydt-history-clear');
    const newestHistory = historyPanel.querySelector('#gydt-history-newest');
    const olderHistory = historyPanel.querySelector('#gydt-history-older');
    const nonce = form.elements._wpnonce.value;
    const limit = 20;
    const uploadIdleMs = 60_000;
    const responseTimeoutMs = 60_000;
    const marks = {
        queued: '·', uploading: '↑', processing: '…', imported: '✓', duplicate: '↪',
        failed: '×', conflict: '!', trash: '–', ready: '–', canceled: '–', timeout: '×', paused: 'Ⅱ',
    };
    let batch = null;
    const completed = new Set();
    let historyDay = '';
    let historyCoverage = null;
    let historyRowsKey = '';
    let olderBefore = 0;
    let historyBusy = false;
    let historyRefreshPending = false;
    let refreshTimer = 0;
    let midnightTimer = 0;

    function updateInputs() {
        const busy = batch !== null;
        input.disabled = busy;
        checkInput.disabled = busy;
        dropArea.setAttribute('aria-disabled', String(busy));
    }

    function uploadFiles(files, checkOnly = false) {
        if (batch || !files.length) {
            return;
        }
        // Keep the File objects in the running batch even after clearing the pickers.
        input.value = '';
        checkInput.value = '';
        list.replaceChildren();
        completed.clear();
        updateHistoryControls();
        if (files.length > limit) {
            message.textContent = 'No ZIPs were processed. Choose up to 20 and try again.';
            return;
        }
        const items = files.map(makeRow);
        batch = {items, index: 0, checkOnly};
        updateInputs();
        advance(batch);
    }

    function makeRow(file) {
        const row = document.createElement('li');
        const summary = document.createElement('div');
        const icon = document.createElement('span');
        const fileInfo = document.createElement('div');
        const heading = document.createElement('div');
        const name = document.createElement('strong');
        const context = document.createElement('span');
        const filename = document.createElement('div');
        const status = document.createElement('div');
        const label = document.createElement('span');
        const detail = document.createElement('span');
        const actions = document.createElement('div');
        const action = document.createElement('span');
        const retryAction = document.createElement('span');
        const progressLine = document.createElement('div');
        const progress = document.createElement('progress');
        const amount = document.createElement('span');

        row.className = 'gydt-queue-row';
        summary.className = 'gydt-row-summary';
        icon.className = 'gydt-row-icon';
        icon.setAttribute('aria-hidden', 'true');
        name.className = 'gydt-row-file';
        name.textContent = file.name;
        heading.className = 'gydt-row-heading';
        context.className = 'gydt-row-context';
        context.hidden = true;
        filename.className = 'gydt-row-context';
        filename.hidden = true;
        heading.append(name, context);
        status.className = 'gydt-row-status';
        status.setAttribute('role', 'status');
        status.setAttribute('aria-label', 'Upload status for ' + file.name);
        status.tabIndex = -1;
        label.className = 'gydt-row-label';
        detail.className = 'gydt-row-detail';
        status.append(label, ' ', detail);
        fileInfo.append(heading, status, filename);
        actions.className = 'gydt-row-actions';
        actions.append(action, retryAction);
        summary.append(icon, fileInfo, actions);

        progressLine.className = 'gydt-row-progress';
        progressLine.hidden = true;
        progress.max = 100;
        amount.setAttribute('aria-hidden', 'true');
        progressLine.append(progress, amount);
        row.append(summary, progressLine);
        const entry = {
            file, row, icon, name, context, filename, action, status, label, detail, retryAction, progressLine, progress, amount,
        };
        setStatus(entry, 'queued', 'Queued');
        list.append(row);
        return entry;
    }

    function setStatus(entry, state, label, detail = '') {
        entry.row.dataset.state = state;
        entry.icon.textContent = marks[state];
        entry.label.textContent = label;
        entry.detail.textContent = detail;
    }

    function showSample(entry, sample) {
        if (typeof sample?.submission_number !== 'string' || !sample.submission_number) {
            return;
        }
        const {submission_number: title, analysis_date: date, sample_number: number} = sample;
        const standard = typeof number === 'string' && /^[0-9]{3,}$/.test(number);
        const details = standard ? [title] : [];
        if (typeof date === 'string' && date && (!standard || !title.includes('-' + date + '-'))) {
            details.push('Analysis date ' + date);
        }
        entry.name.textContent = standard ? number : title;
        entry.name.classList.toggle('gydt-row-number', standard);
        entry.context.textContent = details.join(' · ');
        entry.context.hidden = !details.length;
        entry.filename.textContent = 'ZIP: ' + entry.file.name;
        entry.filename.hidden = false;
        entry.status.setAttribute('aria-label', 'Upload status for ' + title);
    }

    function reviewLink(url, title) {
        const link = document.createElement('a');
        link.className = 'button button-small';
        link.href = url;
        link.target = '_blank';
        link.rel = 'noopener';
        link.textContent = 'Review draft';
        link.setAttribute('aria-label', 'Review draft for ' + title + ' (opens in a new tab)');
        return link;
    }

    function hideButton(title, hide) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'button button-small gydt-hide-upload';
        button.textContent = '×';
        button.setAttribute('aria-label', 'Hide upload for ' + title);
        button.title = 'Hide this upload from your list';
        button.addEventListener('click', hide);
        return button;
    }

    function removeCompleted(entries) {
        for (const entry of entries) {
            if (completed.delete(entry)) {
                entry.row.remove();
            }
        }
        updateHistoryControls();
    }

    function hideResult(result) {
        const matching = [...completed].filter(entry => entry.result.id === result.id);
        if (result.history_day && (!historyDay || result.history_day === historyDay)) {
            void syncHistory({operation: 'dismiss', id: result.id, day: result.history_day, hide: matching});
        } else {
            removeCompleted(matching);
            historyMessage.textContent = 'Upload hidden from your list.';
            historyMessage.focus({preventScroll: true});
        }
    }

    function renderCompleted(entry, current) {
        const title = current?.title ?? entry.file.name;
        const status = current?.status ?? 'Current status unavailable. Use Refresh status to check again.';
        entry.clearKey = current ? entry.result.clear_key : null;
        const key = JSON.stringify([title, status, current?.review_url]);
        if (entry.displayedResult === key) {
            return;
        }
        entry.displayedResult = key;
        entry.label.textContent = entry.state === 'duplicate' ? 'Already imported · ' + status
            : status === 'Draft' ? 'Draft saved' : status;
        entry.detail.replaceChildren();
        if (current?.review_url) {
            entry.detail.append(reviewLink(current.review_url, title), ' ');
        }
        entry.detail.append(hideButton(title, () => hideResult(entry.result)));
    }

    function updateHistoryControls() {
        refreshHistory.disabled = historyBusy;
        newestHistory.disabled = historyBusy;
        olderHistory.disabled = historyBusy;
        clearHistory.disabled = historyBusy || !historyDay || (!historyCoverage && completed.size === 0);
        for (const button of document.querySelectorAll('.gydt-hide-upload')) {
            button.disabled = historyBusy;
        }
    }

    function renderHistory(data, before) {
        if (historyDay !== data.day || !before || !historyCoverage) {
            historyCoverage = data.clear;
        }
        historyDay = data.day;
        olderBefore = data.next_before;
        newestHistory.hidden = !before;
        olderHistory.hidden = !olderBefore;
        const rowsKey = JSON.stringify(data.rows);
        if (rowsKey !== historyRowsKey) {
            historyRowsKey = rowsKey;
            historyList.replaceChildren(...data.rows.map(makeHistoryRow));
        }
        historyMessage.textContent = data.rows.length ? '' : 'No uploads on this page.';
        clearTimeout(midnightTimer);
        midnightTimer = setTimeout(() => {
            historyDay = '';
            historyCoverage = null;
            historyRowsKey = '';
            historyList.replaceChildren();
            newestHistory.hidden = true;
            olderHistory.hidden = true;
            updateHistoryControls();
            queueHistoryRefresh();
        }, data.seconds_to_midnight * 1000 + 100);

        function makeHistoryRow(result) {
            const row = document.createElement('li');
            row.className = 'gydt-queue-row gydt-history-row';
            const title = document.createElement('strong');
            title.className = 'gydt-row-file';
            title.textContent = result.title;
            const status = document.createElement('span');
            status.className = 'gydt-history-status';
            status.textContent = result.status;
            const actions = document.createElement('div');
            actions.className = 'gydt-history-actions';
            if (result.review_url) {
                actions.append(reviewLink(result.review_url, result.title));
            }
            actions.append(hideButton(result.title, () => hideResult({...result, history_day: data.day})));
            row.append(title, status, actions);
            return row;
        }
    }

    async function syncHistory({operation = 'list', id = 0, day = historyDay, hide = [], before = 0, clearKey = ''} = {}) {
        if (historyBusy) {
            historyRefreshPending ||= operation === 'list';
            return;
        }
        historyBusy = true;
        updateHistoryControls();
        const sent = [...completed];
        const receipts = Object.fromEntries(sent.map(entry => [entry.result.id, entry.result.key]));
        const body = new FormData();
        for (const [key, value] of Object.entries({_wpnonce: historyPanel.dataset.nonce, operation, id, day, before, clear_key: clearKey,
            receipts: JSON.stringify(receipts)})) {
            body.set(key, value);
        }
        const controller = new AbortController();
        const deadline = setTimeout(() => controller.abort(), 10_000);
        try {
            const response = await fetch(historyPanel.dataset.url, {method: 'POST', body, signal: controller.signal});
            const data = await response.json();
            if (!response.ok) {
                throw new Error(typeof data.message === 'string' ? data.message : 'Couldn’t load upload history. Use Refresh status to try again.');
            }
            if (typeof data.day !== 'string' || !Array.isArray(data.rows) || !Array.isArray(data.queue)
                || !Number.isInteger(data.seconds_to_midnight) || data.seconds_to_midnight < 1 || data.seconds_to_midnight > 90_000) {
                throw new Error('Couldn’t load upload history. Use Refresh status to try again.');
            }
            const queue = new Map(data.queue.map(result => [result.id, result]));
            for (const entry of sent.filter(entry => completed.has(entry))) {
                renderCompleted(entry, queue.get(entry.result.id));
            }
            renderHistory(data, day === data.day ? before : 0);
            if (operation !== 'list') {
                if (data.changed) {
                    removeCompleted(hide);
                    historyMessage.textContent = operation === 'clear' ? 'Uploads hidden from your list.' : 'Upload hidden from your list.';
                } else {
                    historyMessage.textContent = 'It’s a new day in Vancouver. You’re seeing today’s uploads.';
                }
                historyMessage.focus({preventScroll: true});
            }
        } catch (error) {
            historyMessage.textContent = error instanceof SyntaxError || error.name === 'AbortError' || error instanceof TypeError
                ? 'Couldn’t load upload history. Use Refresh status to try again.' : error.message;
        } finally {
            clearTimeout(deadline);
            historyBusy = false;
            updateHistoryControls();
            if (historyRefreshPending) {
                historyRefreshPending = false;
                queueHistoryRefresh();
            }
        }
    }

    function queueHistoryRefresh() {
        if (document.hidden || refreshTimer) {
            return;
        }
        refreshTimer = setTimeout(() => {
            refreshTimer = 0;
            void syncHistory();
        }, 200);
    }

    refreshHistory.addEventListener('click', () => void syncHistory());
    newestHistory.addEventListener('click', () => void syncHistory());
    olderHistory.addEventListener('click', () => void syncHistory({before: olderBefore}));
    clearHistory.addEventListener('click', () => {
        const snapshot = [...completed];
        let coverage = historyCoverage;
        for (const entry of snapshot) {
            if (entry.result.history_day === historyDay && entry.clearKey && entry.result.id > (coverage?.id ?? 0)) {
                coverage = {id: entry.result.id, key: entry.clearKey};
            }
        }
        if (coverage) {
            void syncHistory({operation: 'clear', id: coverage.id, clearKey: coverage.key, hide: snapshot});
        } else {
            removeCompleted(snapshot);
            historyMessage.textContent = 'Uploads hidden from your list.';
            historyMessage.focus({preventScroll: true});
        }
    });
    document.addEventListener('visibilitychange', queueHistoryRefresh);
    window.addEventListener('focus', queueHistoryRefresh);

    input.addEventListener('change', () => uploadFiles(Array.from(input.files)));
    checkInput.addEventListener('change', () => uploadFiles(Array.from(checkInput.files), true));

    dropArea.addEventListener('dragover', event => {
        event.preventDefault();
        const transfer = event.dataTransfer;
        const acceptsFiles = !batch && transfer?.types.includes('Files');
        if (transfer) {
            transfer.dropEffect = acceptsFiles ? 'copy' : 'none';
        }
        dropArea.classList.toggle('is-dragging', Boolean(acceptsFiles));
    });
    dropArea.addEventListener('dragleave', event => {
        if (!dropArea.contains(event.relatedTarget)) {
            dropArea.classList.remove('is-dragging');
        }
    });
    dropArea.addEventListener('drop', event => {
        event.preventDefault();
        dropArea.classList.remove('is-dragging');
        uploadFiles(Array.from(event.dataTransfer?.files ?? []));
    });

    function disableAction(button, entry) {
        const hadFocus = document.activeElement === button;
        button.disabled = true;
        if (hadFocus) {
            entry.status.focus({preventScroll: true});
        }
    }

    function readResult(payload, importing) {
        const expected = [importing ? 'imported' : 'ready', 'duplicate', 'trash', 'conflict', 'failed', 'unknown'];
        try {
            const result = JSON.parse(payload);
            if (result && expected.includes(result.state) && typeof result.message === 'string') {
                return result;
            }
        } catch {
            // A complete HTTP response without an importer result leaves the outcome uncertain.
        }
        return {state: 'unconfirmed'};
    }

    function send(entry, mode) {
        const importing = mode === 'import';
        const phase = importing ? 'validate the ZIP and save the draft' : 'check for an existing draft';
        setStatus(entry, 'uploading', 'Uploading', 'To ' + phase + '.');
        entry.progress.setAttribute('aria-label', 'Upload to ' + phase + ' for ' + entry.file.name);
        entry.progress.removeAttribute('value');
        entry.progress.removeAttribute('aria-valuetext');
        entry.amount.textContent = '';
        entry.progressLine.hidden = false;
        entry.progressLine.style.visibility = '';
        const body = new FormData();
        body.set('_wpnonce', nonce);
        body.set('mode', mode);
        body.set('sample_zip', entry.file);

        const request = new XMLHttpRequest();
        const cancel = document.createElement('button');
        cancel.type = 'button';
        cancel.className = 'button button-small';
        cancel.textContent = 'Cancel upload';
        cancel.setAttribute('aria-label', 'Cancel upload for ' + entry.file.name);

        return new Promise(resolve => {
            let deadline;
            let uploaded = 0;
            let uploadComplete = false;
            let settled = false;

            function processing() {
                if (entry.row.dataset.state !== 'processing') {
                    setStatus(entry, 'processing', 'Processing', 'Waiting for WordPress to ' + phase + '.');
                }
            }

            function showAmount(event) {
                if (event.lengthComputable && event.total > 0) {
                    const percent = Math.min(100, event.loaded / event.total * 100);
                    entry.progress.value = percent;
                    entry.amount.textContent = Math.floor(percent) + '%';
                    if (percent === 100) {
                        processing();
                    }
                }
            }

            function finish(result, abort = false) {
                if (settled) {
                    return;
                }
                settled = true;
                clearTimeout(deadline);
                disableAction(cancel, entry);
                // Keep the old control's space so the next row cannot move under a repeated click.
                cancel.style.visibility = 'hidden';
                entry.progressLine.style.visibility = 'hidden';
                if (abort) {
                    request.abort();
                }
                resolve(result);
            }

            function armDeadline(milliseconds) {
                clearTimeout(deadline);
                const timer = setTimeout(() => {
                    if (deadline === timer) {
                        finish({state: 'timeout'}, true);
                    }
                }, milliseconds);
                deadline = timer;
            }

            // Each request gets its own button and transport, including during Retry.
            cancel.addEventListener('click', () => {
                if (!settled) {
                    finish({state: 'canceled'}, true);
                }
            });
            entry.action.replaceChildren(cancel);

            request.upload.addEventListener('progress', event => {
                if (!settled && !uploadComplete && event.loaded > uploaded) {
                    uploaded = event.loaded;
                    armDeadline(uploadIdleMs);
                    showAmount(event);
                }
            });
            request.upload.addEventListener('load', event => {
                if (!settled && !uploadComplete) {
                    uploadComplete = true;
                    armDeadline(responseTimeoutMs);
                    showAmount(event);
                    const knownLength = entry.progress.hasAttribute('value');
                    entry.progress.value = 100;
                    if (knownLength) {
                        entry.amount.textContent = '100%';
                    } else {
                        entry.progress.setAttribute('aria-valuetext', 'Upload complete');
                    }
                    processing();
                }
            });
            request.addEventListener('load', () => finish(readResult(request.responseText, importing)));
            request.addEventListener('error', () => finish({state: 'disconnected'}));
            request.addEventListener('abort', () => finish({state: 'disconnected'}));

            try {
                request.open('POST', form.action);
                armDeadline(uploadIdleMs);
                request.send(body);
            } catch {
                finish({state: 'disconnected'});
            }
        });
    }

    function pause(current, entry, outcome, checking) {
        const reason = {
            disconnected: 'The connection was interrupted.',
            unconfirmed: 'WordPress didn’t confirm the request.',
            timeout: 'The retry timed out.',
        }[outcome.state] ?? outcome.message;
        const detail = current.checkOnly
            ? 'The check didn’t finish. No import was attempted.'
            : checking
                ? 'The earlier import is still unconfirmed. Choose Retry to check again.'
                : reason + ' A draft may have saved. Choose Retry to check before uploading again.';
        setStatus(entry, 'paused', 'Paused', detail);
        message.textContent = 'Batch paused on ZIP ' + (current.index + 1) + ' of ' + current.items.length
            + '. Before reloading, note this ZIP and any files still marked Queued.';

        const retry = document.createElement('button');
        retry.type = 'button';
        retry.className = 'button button-small';
        retry.textContent = 'Retry';
        retry.setAttribute('aria-label', 'Retry upload for ' + entry.file.name);
        const index = current.index;
        retry.addEventListener('click', event => {
            if (event.detail > 1 || retry.disabled || batch !== current || current.index !== index) {
                return;
            }
            disableAction(retry, entry);
            // The separate slot keeps a repeated Retry click away from Cancel upload.
            retry.style.visibility = 'hidden';
            advance(current, true);
        });
        entry.retryAction.replaceChildren(retry);
    }

    function showOutcome(entry, result, checkOnly) {
        if (result.state === 'canceled' || result.state === 'timeout') {
            const canceled = result.state === 'canceled';
            entry.state = checkOnly ? 'unchecked' : 'unknown';
            setStatus(entry, result.state, canceled ? 'Canceled' : 'Timed out',
                checkOnly ? 'The check didn’t finish. No import was attempted.'
                    : 'A draft may have saved. Use Check ZIPs without importing to check this ZIP.');
            return;
        }

        entry.state = result.state;
        showSample(entry, result.sample);
        const label = {
            imported: 'Draft saved', duplicate: 'Already imported', trash: 'In Trash', conflict: 'Needs review',
            ready: 'Ready to import', failed: checkOnly ? 'Check failed' : 'Failed',
        }[result.state];
        const detail = result.state === 'ready' ? 'No draft was created. Use Choose sample ZIPs to import this file.'
            : ['imported', 'duplicate'].includes(result.state) ? '' : result.message;
        setStatus(entry, result.state, label, detail);
        if (['imported', 'duplicate'].includes(result.state) && result.result) {
            entry.result = result.result;
            completed.add(entry);
            renderCompleted(entry, entry.result);
            updateHistoryControls();
            queueHistoryRefresh();
        }
    }

    async function advance(current, retrying = false) {
        for (; current.index < current.items.length; current.index++) {
            const entry = current.items[current.index];
            message.textContent = 'ZIP ' + (current.index + 1) + ' of ' + current.items.length;
            let checking = current.checkOnly || retrying;
            let result = await send(entry, checking ? 'preview' : 'import');
            if (!current.checkOnly && result.state === 'ready') {
                showSample(entry, result.sample);
                checking = false;
                result = await send(entry, 'import');
            }
            if (['disconnected', 'unconfirmed', 'unknown'].includes(result.state)
                || (!current.checkOnly && checking && result.state === 'failed') || (retrying && result.state === 'timeout')) {
                pause(current, entry, result, checking);
                return;
            }
            showOutcome(entry, result, current.checkOnly);
            retrying = false;
        }

        if (current.items.some(entry => entry.state === 'unknown')) {
            message.textContent = 'Some ZIPs may have saved. Use Check ZIPs without importing to check those files.';
        } else {
            message.textContent = current.checkOnly ? 'Check finished. No import was attempted.' : 'Batch finished. Check each ZIP’s status below.';
        }
        for (const entry of current.items) {
            entry.progressLine.hidden = true;
            entry.action.replaceChildren();
            entry.retryAction.replaceChildren();
        }
        batch = null;
        updateInputs();
    }

    form.addEventListener('submit', event => event.preventDefault());
    queueHistoryRefresh();
})();
