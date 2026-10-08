'use strict';

// No framework or build step: the server embeds these assets in its executable.
const $ = id => document.getElementById(id);
const state = { chats: [], active: null, mode: 'chat', refs: [], stream: null, runtime: null,
  sending: false, uploading: false, navigating: false, online: false, drafts: new Map(), live: new Map(), preview: null };
let toastTimer, refreshTimer;

function node(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

function toast(message) {
  $('toast').textContent = message;
  $('toast').hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $('toast').hidden = true; }, 5500);
}

async function api(path, { method = 'GET', body } = {}) {
  const headers = {};
  if (method !== 'GET') headers['X-Pocket-Request'] = '1';
  if (body && !(body instanceof FormData)) { headers['Content-Type'] = 'application/json'; body = JSON.stringify(body); }
  const response = await fetch(path, { method, headers, body, credentials: 'same-origin' });
  let result;
  try { result = await response.json(); } catch { throw new Error(`Server returned ${response.status}. Check your connection.`); }
  if (!response.ok) {
    if (response.status === 401 && path !== '/api/login') showLogin();
    throw new Error(result.error || `Request failed (${response.status}).`);
  }
  return result;
}

function showLogin() {
  state.stream?.close(); state.stream = null;
  state.active = null; state.chats = []; state.refs = [];
  state.drafts.clear(); state.live.clear(); $('prompt').value = '';
  for (const dialog of document.querySelectorAll('dialog[open]')) dialog.close();
  $('full-image').removeAttribute('src');
  $('desk').hidden = true; $('login-screen').hidden = false;
}

async function openDesk() {
  $('login-screen').hidden = true; $('desk').hidden = false;
  await refreshAll();
  if (state.chats.length) await selectChat(state.chats[0].id);
  else render();
  connectEvents();
}

async function refreshAll() {
  const [chats, status] = await Promise.all([api('/api/chats'), api('/api/status')]);
  state.chats = chats; state.runtime = status.runtime;
  renderSidebar(); renderStatus();
  if (state.active) await refreshActive();
}

async function refreshActive() {
  const id = state.active?.id;
  if (!id) return;
  const chat = await api(`/api/chats/${id}`);
  if (state.active?.id !== id) return; // A slow response cannot replace another project.
  if (!chat.jobs.some(j => ['queued', 'running'].includes(j.status))) {
    for (const message of chat.messages) state.live.delete(message.id);
  } else {
    for (const message of chat.messages) {
      if (state.live.has(message.id)) message.text = state.live.get(message.id);
    }
  }
  state.active = chat;
  render();
}

function scheduleRefresh() {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => refreshAll().catch(error => toast(error.message)), 120);
}

function rememberDraft() {
  if (state.active) state.drafts.set(state.active.id, { text: $('prompt').value, refs: [...state.refs], mode: state.mode });
}

async function selectChat(id) {
  rememberDraft();
  state.navigating = true; renderControls();
  try {
    const chat = await api(`/api/chats/${id}`);
    state.active = chat;
    const draft = state.drafts.get(id);
    $('prompt').value = draft?.text || ''; state.refs = draft?.refs || [];
    setMode(draft?.mode || 'chat');
    render();
    $('sidebar').classList.remove('open'); $('menu-button').setAttribute('aria-expanded', 'false');
  } finally { state.navigating = false; renderControls(); }
}

async function newChat() {
  rememberDraft();
  state.navigating = true; renderControls();
  try {
    const chat = await api('/api/chats', { method: 'POST', body: { title: 'Untitled project' } });
    state.active = chat; state.refs = []; $('prompt').value = ''; setMode('chat');
    state.chats.unshift({ id: chat.id, title: chat.title, preview: '', updated_at: chat.updated_at });
    render(); $('prompt').focus();
    $('sidebar').classList.remove('open'); $('menu-button').setAttribute('aria-expanded', 'false');
    return chat;
  } finally { state.navigating = false; renderControls(); }
}

function busyJob() { return state.active?.jobs.find(j => ['queued', 'running'].includes(j.status)); }
function imageUrl(media, download = false) { return `/api/chats/${state.active.id}/media/${media.id}${download ? '?download=1' : ''}`; }
function imageById(id) { return state.active?.media.find(m => m.id === id); }

function renderSidebar() {
  $('project-count').textContent = state.chats.length;
  const list = $('chat-list'); list.replaceChildren();
  if (!state.chats.length) list.append(node('p', 'chat-link-preview', 'Your next idea starts here.'));
  for (const chat of state.chats) {
    const button = node('button', `chat-link${state.active?.id === chat.id ? ' active' : ''}`);
    if (state.active?.id === chat.id) button.setAttribute('aria-current', 'page');
    button.append(node('span', 'chat-link-title', chat.title), node('span', 'chat-link-preview', chat.busy ? 'Working on it…' : chat.preview || 'A fresh canvas'));
    button.addEventListener('click', () => selectChat(chat.id).catch(error => toast(error.message)));
    list.append(button);
  }
}

// Text from the model and user never goes through innerHTML. Minimal formatting
// keeps briefs readable without introducing a Markdown dependency or HTML injection.
function renderText(element, text) {
  element.replaceChildren();
  for (const block of text.split(/\n\n+/)) {
    const heading = /^#{1,3}\s+(.+)$/s.exec(block);
    const paragraph = node(heading ? 'h3' : 'p');
    const content = heading ? heading[1] : block;
    const tokens = content.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
    for (const token of tokens) {
      if (token.startsWith('**') && token.endsWith('**')) paragraph.append(node('strong', '', token.slice(2, -2)));
      else if (token.startsWith('`') && token.endsWith('`')) paragraph.append(node('code', '', token.slice(1, -1)));
      else paragraph.append(document.createTextNode(token));
    }
    element.append(paragraph);
  }
}

function renderMessages() {
  const list = $('message-list');
  const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 110;
  const previousTop = list.scrollTop;
  list.replaceChildren();
  const messages = state.active?.messages || [];
  $('welcome').hidden = messages.length > 0;
  list.hidden = messages.length === 0;
  for (const message of messages) {
    const row = node('article', `message ${message.role}`); row.dataset.messageId = message.id;
    row.append(node('div', 'message-avatar', message.role === 'user' ? 'YOU' : '✳'));
    const body = node('div', 'message-body');
    const heading = node('div', 'message-heading', message.role === 'user' ? 'You' : 'Pocket Codex');
    const time = node('time', '', new Date(message.created_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }));
    time.dateTime = new Date(message.created_at * 1000).toISOString(); heading.append(time);
    const copy = node('div', 'message-copy');
    renderText(copy, message.text || (busyJob() ? 'Thinking…' : ''));
    body.append(heading, copy);
    if (message.media_ids.length) {
      const images = node('div', 'message-images');
      for (const id of message.media_ids) {
        const media = imageById(id); if (!media) continue;
        const button = node('button', 'message-image'); button.setAttribute('aria-label', `Open ${media.name}`);
        const img = node('img'); img.src = imageUrl(media); img.alt = media.name; img.loading = 'lazy';
        button.append(img); button.addEventListener('click', () => preview(media)); images.append(button);
      }
      body.append(images);
    }
    row.append(body); list.append(row);
  }
  list.scrollTop = nearBottom ? list.scrollHeight : previousTop;
}

function renderGallery() {
  const media = state.active?.media || [];
  $('image-count').textContent = media.length;
  $('gallery-empty').hidden = media.length > 0;
  $('gallery-list').hidden = media.length === 0;
  $('export-chat').disabled = !state.active;
  const list = $('gallery-list'); list.replaceChildren();
  for (const image of [...media].reverse()) {
    const figure = node('figure', 'gallery-card');
    const button = node('button', 'gallery-preview'); button.setAttribute('aria-label', `Open ${image.name}`);
    const img = node('img'); img.src = imageUrl(image); img.alt = image.name; img.loading = 'lazy';
    button.append(img); button.addEventListener('click', () => preview(image));
    const caption = node('figcaption'); caption.append(node('span', '', image.name), node('small', 'media-kind', image.kind === 'generated' ? 'Created' : 'Reference'));
    const actions = node('div', 'gallery-card-actions');
    const reuse = node('button', '', 'Use as reference'); reuse.addEventListener('click', () => attachExisting(image));
    const download = node('a', '', 'Download ↗'); download.href = imageUrl(image, true); download.download = image.filename;
    actions.append(reuse, download); figure.append(button, caption, actions); list.append(figure);
  }
}

function renderAttachments() {
  const container = $('attachments'); container.replaceChildren(); container.hidden = state.refs.length === 0;
  for (const id of state.refs) {
    const media = imageById(id); if (!media) continue;
    const wrapper = node('div', 'attachment');
    const img = node('img'); img.src = imageUrl(media); img.alt = media.name;
    const remove = node('button', '', '×'); remove.type = 'button'; remove.setAttribute('aria-label', `Remove ${media.name} from this message`);
    remove.addEventListener('click', () => { state.refs = state.refs.filter(ref => ref !== id); rememberDraft(); renderAttachments(); });
    wrapper.append(img, remove); container.append(wrapper);
  }
}

function renderControls() {
  const job = busyJob();
  $('job-bar').hidden = !job;
  $('job-label').textContent = job?.status === 'queued' ? 'Queued — waiting for the laptop…' : 'Working on it…';
  $('cancel-job').disabled = false;
  const latest = state.active?.jobs.at(-1);
  $('job-error').hidden = !latest?.error;
  $('job-error').textContent = latest?.error || '';
  const navigating = state.sending || state.uploading || state.navigating;
  $('send-button').disabled = Boolean(job || navigating || !$('prompt').value.trim());
  $('upload-button').disabled = navigating;
  $('new-chat').disabled = navigating;
  for (const button of $('chat-list').querySelectorAll('button')) button.disabled = navigating;
  $('composer-hint').textContent = state.uploading ? 'Uploading your reference…' : state.runtime?.mock ? 'Mock mode · previews only, no AI usage.' : 'Reference images welcome. PNG, JPEG or WebP.';
}

function renderStatus() {
  const runtime = state.runtime;
  const ready = state.online && runtime?.ready;
  $('status-dot').className = `status-dot ${ready ? 'ready' : 'error'}`;
  $('status-label').textContent = !state.online ? 'Reconnecting' : runtime?.mock ? 'Mock mode' : runtime?.ready ? 'Laptop ready' : 'Needs attention';
  $('status-detail').textContent = runtime?.mock
    ? 'Mock mode is active. Chat replies and images are fixtures so you can explore the UI without using Codex.'
    : runtime?.error || (runtime?.ready ? `Codex is connected.${runtime.image_generation === false ? ' Your provider reports image generation unavailable.' : runtime.image_generation === true ? ' Image generation is available.' : ' Image-generation access will be checked when you try it.'}` : 'Connecting to Codex on the laptop…');
}

function render() {
  $('project-label').textContent = state.active?.title || 'A fresh start';
  renderSidebar(); renderMessages(); renderGallery(); renderAttachments(); renderControls(); renderStatus();
}

function setMode(mode) {
  state.mode = mode;
  for (const value of ['chat', 'generate']) {
    $(`mode-${value}`).classList.toggle('selected', mode === value);
    $(`mode-${value}`).setAttribute('aria-pressed', String(mode === value));
  }
  $('prompt').placeholder = mode === 'generate' ? 'Describe the image you want to make…' : 'What are we creating today?';
  rememberDraft();
}

function attachExisting(media) {
  if (state.refs.includes(media.id)) { toast('This image is already attached.'); return; }
  if (state.refs.length >= 5) { toast('You can attach up to five references.'); return; }
  state.refs.push(media.id); rememberDraft(); renderAttachments(); $('prompt').focus();
  toast('Image attached to your next message.');
}

function preview(media) {
  state.preview = media;
  $('full-image').src = imageUrl(media); $('full-image').alt = media.name;
  $('image-dialog-name').textContent = media.name;
  $('download-image').href = imageUrl(media, true); $('download-image').download = media.filename;
  $('image-dialog').showModal();
}

function connectEvents() {
  state.stream?.close();
  const stream = new EventSource('/api/events'); state.stream = stream;
  stream.onopen = () => { state.online = true; renderStatus(); };
  stream.onerror = () => { state.online = false; renderStatus(); };
  stream.addEventListener('update', event => {
    const data = JSON.parse(event.data);
    if (['resync', 'status'].includes(data.type)) { scheduleRefresh(); return; }
    if (data.type === 'refresh') { scheduleRefresh(); return; }
    if (data.chat_id !== state.active?.id) return;
    if (data.type === 'progress') $('job-label').textContent = data.text;
    if (data.type === 'text') {
      state.live.set(data.message_id, data.text);
      let message = state.active.messages.find(m => m.id === data.message_id);
      if (!message) {
        message = { id: data.message_id, role: 'assistant', text: data.text, media_ids: [], created_at: Math.floor(Date.now() / 1000), job_id: data.job_id };
        state.active.messages.push(message); renderMessages();
      }
      message.text = data.text;
      const list = $('message-list');
      const nearBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 110;
      const row = [...list.children].find(el => el.dataset.messageId === data.message_id);
      if (row) renderText(row.querySelector('.message-copy'), data.text);
      if (nearBottom) list.scrollTop = list.scrollHeight;
    }
  });
}

$('login-form').addEventListener('submit', async event => {
  event.preventDefault(); $('login-error').textContent = ''; $('login-button').disabled = true;
  try {
    await api('/api/login', { method: 'POST', body: { key: $('access-key').value } });
    $('access-key').value = ''; await openDesk();
  } catch (error) { $('login-error').textContent = error.message; }
  finally { $('login-button').disabled = false; }
});
$('logout').addEventListener('click', async () => {
  try { await api('/api/logout', { method: 'POST' }); showLogin(); }
  catch (error) { toast(error.message); }
});
$('new-chat').addEventListener('click', () => newChat().catch(error => toast(error.message)));
$('mode-chat').addEventListener('click', () => setMode('chat'));
$('mode-generate').addEventListener('click', () => setMode('generate'));
$('prompt').addEventListener('input', () => { rememberDraft(); renderControls(); });
$('prompt').addEventListener('keydown', event => {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && window.innerWidth > 700) {
    event.preventDefault(); if (!$('send-button').disabled) $('composer').requestSubmit();
  }
});
$('composer').addEventListener('submit', async event => {
  event.preventDefault();
  const text = $('prompt').value.trim();
  if (!text || busyJob() || state.sending || state.uploading || state.navigating) return;
  state.sending = true; renderControls();
  const mode = state.mode, refs = [...state.refs];
  try {
    if (!state.active) {
      // Create without resetting the text being submitted.
      state.active = await api('/api/chats', { method: 'POST', body: { title: [...text].slice(0, 60).join('').replace(/[\r\n\t]/g, ' ') } });
    } else if (state.active.title === 'Untitled project' && !state.active.messages.length) {
      await api(`/api/chats/${state.active.id}`, { method: 'PATCH', body: { title: [...text].slice(0, 60).join('').replace(/[\r\n\t]/g, ' ') } });
    }
    const id = state.active.id;
    await api(`/api/chats/${id}/messages`, { method: 'POST', body: { text, mode, media_ids: refs } });
    $('prompt').value = ''; state.refs = []; state.drafts.delete(id);
    await refreshAll();
  } catch (error) { toast(error.message); }
  finally { state.sending = false; renderAttachments(); renderControls(); }
});
$('upload-button').addEventListener('click', () => $('file-input').click());
$('file-input').addEventListener('change', async () => {
  const files = [...$('file-input').files]; $('file-input').value = '';
  if (!files.length) return;
  if (files.length + state.refs.length > 5) { toast('Attach at most five images to a message.'); return; }
  state.uploading = true; renderControls();
  try {
    if (!state.active) {
      const draftText = $('prompt').value; const mode = state.mode;
      await newChat(); $('prompt').value = draftText; setMode(mode);
    }
    const id = state.active.id;
    for (const file of files) {
      if (file.size > 12 * 1024 * 1024) throw new Error(`${file.name} exceeds 12 MB.`);
      if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) throw new Error('Use PNG, JPEG, or WebP images.');
      const form = new FormData(); form.append('file', file);
      const media = await api(`/api/chats/${id}/uploads`, { method: 'POST', body: form });
      if (state.active?.id === id) { state.refs.push(media.id); rememberDraft(); }
    }
    await refreshActive(); renderAttachments();
  } catch (error) { toast(error.message); }
  finally { state.uploading = false; renderControls(); }
});
$('cancel-job').addEventListener('click', async () => {
  const job = busyJob(); if (!job) return;
  $('cancel-job').disabled = true; $('job-label').textContent = 'Stopping…';
  try { await api(`/api/chats/${state.active.id}/jobs/${job.id}/cancel`, { method: 'POST' }); }
  catch (error) { toast(error.message); $('cancel-job').disabled = false; }
});
for (const button of document.querySelectorAll('.starter')) button.addEventListener('click', () => {
  $('prompt').value = button.dataset.prompt; setMode(button.dataset.mode); renderControls(); $('prompt').focus();
});
$('menu-button').addEventListener('click', () => {
  const open = $('sidebar').classList.toggle('open'); $('menu-button').setAttribute('aria-expanded', String(open));
});
$('gallery-toggle').addEventListener('click', () => {
  const gallery = $('gallery');
  if (window.innerWidth <= 950) {
    gallery.hidden = false;
    const open = gallery.classList.toggle('open'); $('gallery-toggle').setAttribute('aria-expanded', String(open));
  } else {
    gallery.hidden = !gallery.hidden; $('gallery-toggle').setAttribute('aria-expanded', String(!gallery.hidden));
  }
});
function syncGalleryState() {
  if (window.innerWidth <= 950) {
    $('gallery').hidden = false;
    $('gallery-toggle').setAttribute('aria-expanded', String($('gallery').classList.contains('open')));
  } else {
    $('gallery-toggle').setAttribute('aria-expanded', String(!$('gallery').hidden));
  }
}
window.addEventListener('resize', syncGalleryState);
syncGalleryState();
$('export-chat').addEventListener('click', () => { if (state.active) window.location.assign(`/api/chats/${state.active.id}/export`); });
$('close-image').addEventListener('click', () => $('image-dialog').close());
$('reference-image').addEventListener('click', () => { if (state.preview) { attachExisting(state.preview); $('image-dialog').close(); setMode('generate'); } });
$('status-button').addEventListener('click', () => { renderStatus(); $('status-dialog').showModal(); });
$('close-status').addEventListener('click', () => $('status-dialog').close());
for (const dialog of document.querySelectorAll('dialog')) dialog.addEventListener('click', event => {
  if (event.target === dialog) { const r = dialog.getBoundingClientRect(); if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) dialog.close(); }
});

// Reconnect events trigger a full snapshot. Poll only as a fallback while disconnected
// or while work is active, so missed events cannot leave a completed job stuck onscreen.
setInterval(() => { if (!$('desk').hidden && (!state.online || busyJob())) refreshAll().catch(() => {}); }, 5000);
document.addEventListener('visibilitychange', () => { if (!document.hidden && !$('desk').hidden) refreshAll().catch(() => {}); });

api('/api/status').then(() => openDesk()).catch(() => { showLogin(); $('access-key').focus(); });
