<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  type Tab = 'library' | 'uploads' | 'permanent' | 'settings' | 'editor';
  type ToastKind = 'ok' | 'err';

  type Clip = {
    id: string;
    filename: string;
    path: string;
    duration: number;
    created_at: string;
    thumbnail_path: string | null;
    tags: string | null;
    folder: string | null;
    upload_status: string;
    r2_key: string | null;
    r2_url: string | null;
    expiry_date: string | null;
    is_permanent: boolean;
  };

  let tab = $state<Tab>('library');
  let clips = $state<Clip[]>([]);
  let uploadedClips = $state<Clip[]>([]);
  let permanentClips = $state<Clip[]>([]);
  let editClip = $state<Clip | null>(null);
  let settings = $state<any>({});
  let recState = $state({ replay_buffer_active: false, recording_active: false });
  let storageUsage = $state(0);
  let thumbCache = $state<Record<string, string>>({});

  let toast = $state<{ msg: string; kind: ToastKind } | null>(null);
  let toastTimer: any;

  let r2TempBytes = $state(0);
  let r2PermBytes = $state(0);

  let videoEl: HTMLVideoElement;
  let previewSrc = $state('');
  let previewLoading = $state(false);
  let currentTime = $state(0);
  let duration = $state(0);
  let trimStart = $state(0);
  let trimEnd = $state(10);

  // ─── Lifecycle ─────────────────────────────────────────────────────────

  onMount(async () => {
    await injectUserTheme();
    settings = await invoke('get_settings');
    await refresh();
    setInterval(refresh, 3000);
    await loadClips();

    listen('clip-saved', (e: any) => {
      notify('Clip saved: ' + e.payload.filename);
      setTimeout(loadClips, 500);
    });
    listen('toast', (e: any) => notify(e.payload.msg, e.payload.kind));
  });

  async function injectUserTheme() {
    const css = await invoke<string>('get_theme_css');
    if (!css) return;
    const el = document.createElement('style');
    el.textContent = css;
    document.head.appendChild(el);
  }

  // ─── Notifications ─────────────────────────────────────────────────────

  function notify(msg: string, kind: ToastKind = 'ok') {
    toast = { msg, kind };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 2500);
  }

  // ─── Data fetching ─────────────────────────────────────────────────────

  async function refresh() {
    recState = await invoke('get_recording_state');
    settings = await invoke('get_settings');
    storageUsage = await invoke('get_storage_usage');
  }

  async function loadClips() {
    clips = await invoke<Clip[]>('scan_clips');
    for (const c of clips) {
      if (c.thumbnail_path) loadThumb(c.thumbnail_path);
    }
  }

  async function loadThumb(path: string) {
    if (thumbCache[path]) return;
    try {
      thumbCache[path] = await invoke<string>('read_thumbnail', { path });
    } catch {}
  }

  async function switchTab(t: Tab) {
    tab = t;
    if (t === 'library') await loadClips();
    if (t === 'uploads') {
      uploadedClips = await invoke('get_uploaded_clips', { permanent: false });
      r2TempBytes = await invoke<number>('r2_storage', { permanent: false }).catch(() => 0);
    }
    if (t === 'permanent') {
      permanentClips = await invoke('get_uploaded_clips', { permanent: true });
      r2PermBytes = await invoke<number>('r2_storage', { permanent: true }).catch(() => 0);
    }
  }

  // ─── Recording controls ────────────────────────────────────────────────

  async function toggleBuffer() {
    try {
      const action = recState.replay_buffer_active ? 'stop_replay_buffer' : 'start_replay_buffer';
      await invoke(action);
      notify(recState.replay_buffer_active ? 'Buffer stopped' : 'Buffer started');
    } catch (e: any) {
      notify(String(e), 'err');
    }
    await refresh();
  }

  // ─── Sharing ───────────────────────────────────────────────────────────

  async function upload(clip: Clip, permanent: boolean) {
    try {
      const url = await invoke<string>('upload_clip', { id: clip.id, permanent });
      await writeText(url);
      notify(permanent ? 'Permanent link copied' : 'Link copied');
    } catch {
      notify('Upload failed', 'err');
    }
  }

  const quickShare = (c: Clip) => upload(c, false);
  const permUpload = (c: Clip) => upload(c, true);

  async function copyLink(clip: Clip) {
    if (!clip.r2_url) return;
    await writeText(clip.r2_url);
    notify('Link copied');
  }

  async function deleteClip(clip: Clip) {
    await invoke('delete_clip', { id: clip.id });
    notify('Deleted');
    await loadClips();
  }

  async function deleteFromR2(clip: Clip) {
    await invoke('delete_from_r2', { id: clip.id });
    notify('Removed from R2');
    await switchTab('permanent');
  }

  // ─── Editor ────────────────────────────────────────────────────────────

  async function openEditor(clip: Clip) {
    editClip = clip;
    trimStart = 0;
    trimEnd = clip.duration || 10;
    tab = 'editor';
    previewSrc = '';
    previewLoading = true;

    try {
      const path = await invoke<string>('transcode_for_preview', { input: clip.path });
      const bytes = await invoke<number[]>('read_video_bytes', { path });
      previewSrc = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'video/mp4' }));
    } catch (e) {
      console.error(e);
      notify('Preview failed', 'err');
    } finally {
      previewLoading = false;
    }
  }

  function closeEditor() {
    if (previewSrc.startsWith('blob:')) URL.revokeObjectURL(previewSrc);
    previewSrc = '';
    editClip = null;
    tab = 'library';
  }

  async function exportTrim(mode: 'copy' | 'overwrite' = 'copy') {
    if (!editClip) return;

    const ext = editClip.path.split('.').pop() || 'mp4';
    const orig = editClip.path;
    const tmp = orig.replace(/\.[^.]+$/, `_tmp_${Date.now()}.${ext}`);
    const final = mode === 'overwrite' ? orig : orig.replace(/\.[^.]+$/, `_trimmed.${ext}`);

    try {
      await invoke('trim_clip', { input: orig, output: tmp, start: trimStart, end: trimEnd });
      await invoke('replace_file', { src: tmp, dst: final });
      notify(mode === 'overwrite' ? 'Clip overwritten' : 'Saved as copy');
      closeEditor();
      setTimeout(loadClips, 500);
    } catch (e) {
      console.error(e);
      notify('Export failed', 'err');
    }
  }

  // ─── Settings ──────────────────────────────────────────────────────────

  async function saveSettings() {
    await invoke('save_settings', { newSettings: settings });
    notify('Settings saved');
  }

  // ─── Formatters ────────────────────────────────────────────────────────

  function fmt(seconds: number) {
    if (!seconds || isNaN(seconds)) return '0:00';
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60).toString().padStart(2, '0');
    return `${m}:${s}`;
  }

  function timeAgo(iso: string) {
    const s = (Date.now() - new Date(iso).getTime()) / 1000;
    if (s < 60) return 'just now';
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
    return `${Math.floor(s / 86400)}d ago`;
  }

  function fmtBytes(b: number) {
    return b < 1e9 ? `${(b / 1e6).toFixed(0)} MB` : `${(b / 1e9).toFixed(1)} GB`;
  }
</script>

{#if toast}
  <div class="toast" class:err={toast.kind==='err'}>
    <span class="dot"></span>{toast.msg}
  </div>
{/if}

<div class="app">
  <header data-tauri-drag-region>
    <div class="brand">
      <img src="/logo.png" alt="klyppd" />
    </div>
    <div class="header-info">
      <button class="rec-btn" class:on={recState.replay_buffer_active} onclick={toggleBuffer}>
        <span class="rec-dot"></span>
        {recState.replay_buffer_active ? 'Recording' : 'Idle'}
      </button>
      <span class="info-pill">{settings.codec || 'h264'} · {settings.fps || 60}fps</span>
      <span class="info-pill">{fmtBytes(storageUsage)}</span>
    </div>
    <div class="window-ctrls">
      <button onclick={() => getCurrentWindow().hide()} aria-label="hide" class="min" title="Hide (run klyppd again to restore)">
        <svg viewBox="0 0 16 16" width="14" height="14"><path stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M4 11h8"/></svg>
      </button>
      <button onclick={() => getCurrentWindow().close()} aria-label="close" class="close">
        <svg viewBox="0 0 16 16" width="14" height="14"><path stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>
  </header>

  <div class="layout">
    <nav class="sidenav">
      <button class:sel={tab==='library'} onclick={() => switchTab('library')}>
        <svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M4 6h16v2H4zm0 5h16v2H4zm0 5h10v2H4z"/></svg>
        <span>Library</span>
      </button>
      <button class:sel={tab==='uploads'} onclick={() => switchTab('uploads')}>
        <svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M19.35 10.04A7.49 7.49 0 0012 4C9.11 4 6.6 5.64 5.35 8.04A6 6 0 006 20h13a5 5 0 00.35-9.96zM14 13v4h-4v-4H7l5-5 5 5h-3z"/></svg>
        <span>Uploads</span>
      </button>
      <button class:sel={tab==='permanent'} onclick={() => switchTab('permanent')}>
        <svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M12 1L3 5v6c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V5l-9-4z"/></svg>
        <span>Permanent</span>
      </button>
      <div class="nav-spacer"></div>
      <button class:sel={tab==='settings'} onclick={() => switchTab('settings')}>
        <svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.488.488 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>
        <span>Settings</span>
      </button>
    </nav>

    <main>
      {#if tab === 'library'}
        <div class="page-header">
          <h1>Library</h1>
          <span class="count">{clips.length} {clips.length === 1 ? 'clip' : 'clips'}</span>
        </div>
        {#if clips.length === 0}
          <div class="empty">
            <svg viewBox="0 0 24 24" width="48" height="48" opacity="0.3"><path fill="currentColor" d="M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"/></svg>
            <h3>No clips yet</h3>
            <p>Press <kbd>{settings.hotkey_save_replay || 'Alt+R'}</kbd> while the buffer is running to save a clip</p>
          </div>
        {:else}
          <div class="grid">
            {#each clips as clip}
              <article class="clip">
                <div class="clip-thumb" role="button" tabindex="0" onclick={() => openEditor(clip)} onkeydown={(e)=>e.key==='Enter'&&openEditor(clip)}>
                  {#if clip.thumbnail_path && thumbCache[clip.thumbnail_path]}
                    <img src={thumbCache[clip.thumbnail_path]} alt="" />
                  {:else}
                    <div class="thumb-placeholder">
                      <svg viewBox="0 0 24 24" width="32" height="32"><path fill="currentColor" d="M8 5v14l11-7z"/></svg>
                    </div>
                  {/if}
                  <span class="clip-duration">{fmt(clip.duration)}</span>
                  <div class="clip-overlay"><span>Edit</span></div>
                </div>
                <div class="clip-meta">
                  <h4 title={clip.filename}>{clip.filename}</h4>
                  <p>{timeAgo(clip.created_at)}</p>
                </div>
                <div class="clip-actions">
                  {#if clip.r2_url}
                    <button class="primary" onclick={() => copyLink(clip)} title={clip.is_permanent ? 'Permanent · click to copy' : 'Temporary · click to copy'}>
                      <svg viewBox="0 0 24 24" width="14" height="14"><path fill="currentColor" d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"/></svg>
                      Copy link
                    </button>
                  {:else}
                    <button class="primary" onclick={() => quickShare(clip)}>
                      <svg viewBox="0 0 24 24" width="14" height="14"><path fill="currentColor" d="M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z"/></svg>
                      Share
                    </button>
                    <button onclick={() => permUpload(clip)} title="Upload permanent">
                      <svg viewBox="0 0 24 24" width="14" height="14"><path fill="currentColor" d="M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1s3.1 1.39 3.1 3.1v2z"/></svg>
                    </button>
                  {/if}
                  <button onclick={() => deleteClip(clip)} title="Delete" class="del">
                    <svg viewBox="0 0 24 24" width="14" height="14"><path fill="currentColor" d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
                  </button>
                </div>
              </article>
            {/each}
          </div>
        {/if}
      {/if}

      {#if tab === 'uploads'}
        <div class="page-header">
          <h1>Temporary Uploads</h1>
          <span class="count">{uploadedClips.length} · {fmtBytes(r2TempBytes)} on R2</span>
        </div>
        {#if uploadedClips.length === 0}
          <div class="empty"><h3>No temporary uploads</h3><p>Upload clips with the Share button</p></div>
        {:else}
          <ul class="rows">
            {#each uploadedClips as clip}
              <li>
                <div><strong>{clip.filename}</strong>{#if clip.expiry_date}<span class="meta-tag">expires {new Date(clip.expiry_date).toLocaleDateString()}</span>{/if}</div>
                <button class="primary" onclick={() => copyLink(clip)}>Copy Link</button>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}

      {#if tab === 'permanent'}
        <div class="page-header">
          <h1>Permanent Uploads</h1>
          <span class="count">{permanentClips.length} · {fmtBytes(r2PermBytes)} on R2</span>
        </div>
        {#if permanentClips.length === 0}
          <div class="empty"><h3>No permanent uploads</h3></div>
        {:else}
          <ul class="rows">
            {#each permanentClips as clip}
              <li>
                <div><strong>{clip.filename}</strong></div>
                <div class="row-buttons">
                  <button onclick={() => copyLink(clip)}>Copy Link</button>
                  <button class="del" onclick={() => deleteFromR2(clip)}>Delete from R2</button>
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}

      {#if tab === 'editor' && editClip}
        <div class="editor">
          <div class="editor-top">
            <button class="back" onclick={() => { tab='library'; editClip=null; if (previewSrc.startsWith('blob:')) URL.revokeObjectURL(previewSrc); previewSrc=''; }}>← Back</button>
            <h2>{editClip.filename}</h2>
            <div class="spacer"></div>
            <button class="ghost" onclick={() => exportTrim('overwrite')} disabled={!previewSrc} title="Replace original with trim">
              Overwrite
            </button>
            <button class="primary" onclick={() => exportTrim('copy')} disabled={!previewSrc}>
              Save Copy ({fmt(trimEnd-trimStart)})
            </button>
          </div>

          <div class="video-container">
            {#if previewLoading}
              <div class="preview-loading">Preparing preview…</div>
            {:else if previewSrc}
              <video bind:this={videoEl} src={previewSrc} controls preload="auto"
                ontimeupdate={() => { if(videoEl) currentTime = videoEl.currentTime; }}
                onloadedmetadata={() => { if(videoEl) { duration = videoEl.duration; if (!trimEnd || trimEnd <= 0) trimEnd = duration; } }}
              ></video>
            {/if}
          </div>

          <div class="timeline-wrap">
            <div class="time-readout">
              <span>{fmt(currentTime)} / {fmt(duration)}</span>
              <span class="trim-info">Trim: {fmt(trimStart)} → {fmt(trimEnd)}</span>
            </div>
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="tl" onclick={(e) => {
              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
              const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
              if (videoEl) videoEl.currentTime = pct * duration;
            }}>
              <div class="tl-track"></div>
              <div class="tl-region" style="left:{(trimStart/(duration||1))*100}%;width:{((trimEnd-trimStart)/(duration||1))*100}%"></div>
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="tl-handle in" style="left:{(trimStart/(duration||1))*100}%"
                onmousedown={(e) => {
                  e.stopPropagation();
                  const rect = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                  const move = (ev: MouseEvent) => {
                    const pct = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
                    trimStart = Math.min(pct * duration, trimEnd - 0.1);
                  };
                  const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); };
                  document.addEventListener('mousemove', move); document.addEventListener('mouseup', up);
                }}
              ></div>
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="tl-handle out" style="left:{(trimEnd/(duration||1))*100}%"
                onmousedown={(e) => {
                  e.stopPropagation();
                  const rect = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
                  const move = (ev: MouseEvent) => {
                    const pct = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width));
                    trimEnd = Math.max(pct * duration, trimStart + 0.1);
                  };
                  const up = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up); };
                  document.addEventListener('mousemove', move); document.addEventListener('mouseup', up);
                }}
              ></div>
              <div class="tl-cursor" style="left:{(currentTime/(duration||1))*100}%"></div>
            </div>
            <div class="tl-actions">
              <button onclick={() => trimStart = currentTime}>← Set In here</button>
              <button onclick={() => trimEnd = currentTime}>Set Out here →</button>
            </div>
          </div>
        </div>
      {/if}

      {#if tab === 'settings'}
        <div class="page-header"><h1>Settings</h1></div>
        <div class="settings">
          <section>
            <h3>Recording</h3>
            <div class="setting"><label>Clips Directory</label><input bind:value={settings.clips_directory} /></div>
            <div class="setting"><label>Buffer Length</label><div class="input-with-suffix"><input type="number" bind:value={settings.buffer_seconds} /><span>seconds</span></div></div>
            <div class="setting"><label>Frame Rate</label><div class="input-with-suffix"><input type="number" bind:value={settings.fps} /><span>fps</span></div></div>
            <div class="setting"><label>Codec</label><select bind:value={settings.codec}><option>h264</option><option>hevc</option><option>av1</option></select></div>
            <div class="setting"><label>Container</label><select bind:value={settings.container}><option>mp4</option><option>mkv</option><option>webm</option></select></div>
            <div class="setting"><label>Audio Source</label><input bind:value={settings.audio_source} /></div>
          </section>
          <section>
            <h3>Hotkeys</h3>
            <div class="setting"><label>Save Replay</label><input bind:value={settings.hotkey_save_replay} /></div>
            <div class="setting"><label>Toggle Recording</label><input bind:value={settings.hotkey_start_stop_recording} /></div>
            <div class="setting"><label>Toggle Buffer</label><input bind:value={settings.hotkey_start_stop_buffer} /></div>
            <p class="hint">Hotkeys are configured in your Hyprland Lua config: <code>~/.config/klyppd/save-replay.sh</code></p>
          </section>
          <section>
            <h3>Cloudflare R2</h3>
            <div class="setting"><label>Endpoint URL</label><input bind:value={settings.r2_endpoint} placeholder="https://…r2.cloudflarestorage.com" /></div>
            <div class="setting"><label>Bucket Name</label><input bind:value={settings.r2_bucket} /></div>
            <div class="setting"><label>Access Key ID</label><input bind:value={settings.r2_access_key} /></div>
            <div class="setting"><label>Secret Key</label><input type="password" bind:value={settings.r2_secret_key} /></div>
            <div class="setting"><label>Custom Domain</label><input bind:value={settings.r2_custom_domain} placeholder="https://cdn.example.com" /></div>
            <div class="setting"><label>Temp Expiry</label><div class="input-with-suffix"><input type="number" bind:value={settings.expiry_days} /><span>days</span></div></div>
          </section>
          <button class="primary save" onclick={saveSettings}>Save Changes</button>
        </div>
      {/if}
    </main>
  </div>
</div>

<style>
  :global(*){box-sizing:border-box;margin:0;padding:0}
  :global(html,body){background:var(--bg-base,#101418);color:var(--text,#e0e2e8);font:13px/1.5 'Inter',-apple-system,'Segoe UI',sans-serif;height:100%;overflow:hidden;-webkit-font-smoothing:antialiased}
  :global(::-webkit-scrollbar){width:8px;height:8px}
  :global(::-webkit-scrollbar-thumb){background:var(--bg-elev-3,#272a2e);border-radius:4px}
  :global(::-webkit-scrollbar-thumb:hover){background:var(--border-strong,#36393e)}

  .app{display:flex;flex-direction:column;height:100vh;background:var(--bg-base,#101418)}

  /* Toast */
  .toast{position:fixed;bottom:20px;left:50%;transform:translateX(-50%);background:var(--bg-elev-2,#1c2024);border:1px solid var(--border-strong,#36393e);color:var(--text,#e0e2e8);padding:10px 16px 10px 14px;border-radius:8px;font-size:12px;z-index:999;display:flex;align-items:center;gap:10px;box-shadow:0 8px 24px rgba(0,0,0,.5);animation:slide .25s ease-out}
  .toast.err{border-color:var(--danger,#ffb4ab);color:var(--danger,#ffb4ab)}
  .toast .dot{width:6px;height:6px;border-radius:50%;background:var(--accent,#9accfa)}
  .toast.err .dot{background:var(--danger,#ffb4ab)}
  @keyframes slide{from{opacity:0;transform:translate(-50%,12px)}to{opacity:1;transform:translate(-50%,0)}}

  /* Header */
  header{display:flex;align-items:center;height:40px;padding:0 14px;background:var(--bg-deepest,#0b0f12);border-bottom:1px solid var(--border,#1c2024);-webkit-app-region:drag;gap:18px}
  .brand{display:flex;align-items:center;gap:9px;-webkit-app-region:drag}
  .brand img{height:24px;width:auto;display:block;opacity:.95}
  .header-info{display:flex;align-items:center;gap:6px;flex:1;-webkit-app-region:no-drag}
  .rec-btn{display:flex;align-items:center;gap:7px;padding:4px 11px;background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);border-radius:6px;font-size:11px;cursor:pointer;transition:all .15s;font-weight:500}
  .rec-btn:hover{border-color:var(--border-strong,#36393e);background:var(--bg-elev-2,#1c2024)}
  .rec-btn.on{border-color:var(--accent-border);color:var(--accent,#9accfa);background:var(--accent-bg)}
  .rec-btn .rec-dot{width:6px;height:6px;border-radius:50%;background:var(--text-faint,#42474e)}
  .rec-btn.on .rec-dot{background:var(--accent,#9accfa);box-shadow:0 0 8px var(--accent,#9accfa);animation:pulse 1.6s ease infinite}
  @keyframes pulse{50%{opacity:.5}}
  .info-pill{font-size:10px;color:var(--text-muted,#8c9198);padding:3px 8px;background:var(--bg-elev-1,#181c20);border-radius:4px;font-family:'JetBrains Mono','Fira Code',ui-monospace,monospace;letter-spacing:.3px}
  .window-ctrls{-webkit-app-region:no-drag;display:flex;gap:2px}
  .window-ctrls button{width:26px;height:26px;background:none;border:none;color:var(--text-muted,#8c9198);border-radius:5px;cursor:pointer;display:flex;align-items:center;justify-content:center;transition:all .12s}
  .window-ctrls button.min:hover{background:var(--bg-elev-2,#1c2024);color:var(--text,#e0e2e8)}
  .window-ctrls button.close:hover{background:var(--danger,#ffb4ab);color:var(--bg-base,#101418)}

  /* Layout */
  .layout{display:flex;flex:1;min-height:0}
  .sidenav{width:168px;background:var(--bg-deepest,#0b0f12);border-right:1px solid var(--border,#1c2024);padding:14px 10px;display:flex;flex-direction:column;gap:1px}
  .sidenav button{display:flex;align-items:center;gap:11px;padding:7px 11px;background:none;border:none;color:var(--text-muted,#8c9198);border-radius:6px;cursor:pointer;font-size:13px;font-weight:500;text-align:left;transition:all .12s;letter-spacing:-.01em}
  .sidenav button:hover{background:var(--bg-elev-1,#181c20);color:var(--text-dim,#c2c7cf)}
  .sidenav button.sel{background:var(--accent-bg);color:var(--accent,#9accfa)}
  .sidenav button svg{opacity:.85}
  .nav-spacer{flex:1}

  main{flex:1;overflow-y:auto;padding:22px 26px}

  /* Page header */
  .page-header{display:flex;align-items:baseline;justify-content:space-between;margin-bottom:18px}
  .page-header h1{font-size:20px;font-weight:600;color:var(--text,#e0e2e8)}
  .count{font-size:12px;color:var(--text-muted,#8c9198)}

  /* Empty state */
  .empty{display:flex;flex-direction:column;align-items:center;justify-content:center;padding:60px 20px;text-align:center;color:var(--text-muted,#8c9198)}
  .empty svg{margin-bottom:14px}
  .empty h3{font-size:15px;font-weight:500;color:var(--text-dim,#c2c7cf);margin-bottom:6px}
  .empty p{font-size:12px}
  .empty kbd{background:var(--bg-elev-2,#1c2024);border:1px solid var(--border-strong,#36393e);padding:2px 6px;border-radius:4px;font-family:'JetBrains Mono',monospace;font-size:11px}

  /* Grid */
  .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:14px}
  .clip{background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);border-radius:10px;overflow:hidden;transition:all .15s}
  .clip:hover{border-color:var(--border-strong,#36393e);transform:translateY(-1px)}
  .clip-thumb{position:relative;aspect-ratio:16/9;background:var(--bg-deepest,#0b0f12);cursor:pointer;overflow:hidden}
  .clip-thumb img{width:100%;height:100%;object-fit:cover;transition:transform .3s}
  .clip-thumb:hover img{transform:scale(1.04)}
  .thumb-placeholder{width:100%;height:100%;display:flex;align-items:center;justify-content:center;color:var(--text-faint,#42474e)}
  .clip-duration{position:absolute;bottom:8px;right:8px;background:rgba(0,0,0,.85);color:#fff;padding:2px 7px;border-radius:4px;font-size:11px;font-weight:500;font-family:'JetBrains Mono',monospace}
  .clip-overlay{position:absolute;inset:0;background:rgba(0,0,0,.6);display:flex;align-items:center;justify-content:center;opacity:0;transition:opacity .15s;color:#fff;font-weight:500;font-size:13px}
  .clip-thumb:hover .clip-overlay{opacity:1}
  .clip-meta{padding:10px 12px 6px}
  .clip-meta h4{font-size:13px;font-weight:500;color:var(--text-dim,#c2c7cf);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .clip-meta p{font-size:11px;color:var(--text-muted,#8c9198);margin-top:2px}
  .clip-actions{display:flex;gap:4px;padding:6px 10px 10px}
  .clip-actions button{display:flex;align-items:center;gap:5px;padding:5px 10px;background:var(--bg-elev-2,#1c2024);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);border-radius:6px;font-size:11px;font-weight:500;cursor:pointer;transition:all .12s}
  .clip-actions button:hover{border-color:var(--border-strong,#36393e);color:var(--text,#e0e2e8)}
  .clip-actions button.primary{background:var(--accent-bg);border-color:var(--accent-border);color:var(--accent,#9accfa);flex:1}
  .clip-actions button.primary:hover{background:rgba(154,204,250,.18)}
  .clip-actions button.del:hover{color:var(--danger,#ffb4ab);border-color:var(--danger,#ffb4ab)}

  /* Rows */
  .rows{list-style:none;display:flex;flex-direction:column;gap:6px}
  .rows li{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 16px;background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);border-radius:8px}
  .rows strong{font-size:13px;font-weight:500;color:var(--text-dim,#c2c7cf)}
  .meta-tag{margin-left:8px;font-size:11px;color:var(--text-muted,#8c9198);font-weight:400}
  .row-buttons{display:flex;gap:6px}
  .rows button{padding:5px 12px;background:var(--bg-elev-2,#1c2024);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);border-radius:6px;font-size:11px;cursor:pointer}
  .rows button:hover{border-color:var(--border-strong,#36393e);color:var(--text,#e0e2e8)}
  .rows button.primary{background:var(--accent-bg);border-color:var(--accent-border);color:var(--accent,#9accfa)}
  .rows button.del:hover{color:var(--danger,#ffb4ab);border-color:var(--danger,#ffb4ab)}

  /* Editor */
  .editor{display:flex;flex-direction:column;gap:14px;height:calc(100vh - 90px);margin:-22px -26px 0;padding:14px 20px;background:#0a0a0a}
  .editor-top{display:flex;align-items:center;gap:14px}
  .editor-top .spacer{flex:1}
  .back{background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);padding:5px 12px;border-radius:6px;font-size:12px;cursor:pointer}
  .back:hover{border-color:var(--border-strong,#36393e)}
  .editor h2{font-size:13px;font-weight:500;color:var(--text-muted,#8c9198);max-width:400px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .editor .primary{padding:7px 16px;background:var(--accent-bg);border:1px solid var(--accent-border);color:var(--accent,#9accfa);border-radius:6px;font-size:12px;font-weight:600;cursor:pointer}
  .editor .primary:hover{background:rgba(154,204,250,.18)}
  .editor .primary:disabled{opacity:.4;cursor:not-allowed}
  .editor .ghost{padding:7px 14px;background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);border-radius:6px;font-size:12px;font-weight:500;cursor:pointer}
  .editor .ghost:hover{border-color:var(--border-strong,#36393e);color:var(--text,#e0e2e8)}
  .editor .ghost:disabled{opacity:.4;cursor:not-allowed}

  .video-container{flex:1;min-height:0;background:#000;border-radius:6px;display:flex;align-items:center;justify-content:center;overflow:hidden;border:1px solid #1a1a1a}
  .video-container video{width:100%;height:100%;object-fit:contain}
  .preview-loading{color:var(--text-muted,#8c9198);font-size:13px;display:flex;align-items:center;gap:10px}
  .preview-loading::before{content:'';width:14px;height:14px;border:2px solid #2a2a2a;border-top-color:var(--accent,#9accfa);border-radius:50%;animation:spin .8s linear infinite}
  @keyframes spin{to{transform:rotate(360deg)}}

  .timeline-wrap{display:flex;flex-direction:column;gap:10px}
  .time-readout{display:flex;justify-content:space-between;font-size:11px;color:var(--text-muted,#8c9198);font-family:'JetBrains Mono',monospace;letter-spacing:.4px}
  .trim-info{color:var(--accent,#9accfa)}

  .tl{position:relative;height:48px;background:#141414;border:1px solid #1f1f1f;border-radius:6px;cursor:pointer;user-select:none}
  .tl-track{position:absolute;inset:0}
  .tl-region{position:absolute;top:0;bottom:0;background:rgba(154,204,250,.08);border-left:3px solid var(--accent,#9accfa);border-right:3px solid var(--accent,#9accfa)}
  .tl-handle{position:absolute;top:-2px;width:16px;height:52px;margin-left:-8px;background:var(--accent,#9accfa);border-radius:4px;cursor:ew-resize;box-shadow:0 0 0 1px #0a0a0a,0 4px 12px rgba(0,0,0,.5);z-index:2;transition:transform .1s}
  .tl-handle:hover{transform:scaleY(1.05)}
  .tl-handle::before{content:'';position:absolute;top:50%;left:50%;transform:translate(-50%,-50%);width:3px;height:18px;background:#0a0a0a;border-radius:2px;box-shadow:5px 0 0 #0a0a0a,-5px 0 0 #0a0a0a}
  .tl-cursor{position:absolute;top:0;bottom:0;width:2px;background:#fff;pointer-events:none;z-index:1;box-shadow:0 0 6px rgba(255,255,255,.5)}

  .tl-actions{display:flex;gap:6px;justify-content:center}
  .tl-actions button{padding:6px 14px;background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);color:var(--text-dim,#c2c7cf);border-radius:6px;font-size:11px;cursor:pointer;font-weight:500}
  .tl-actions button:hover{border-color:var(--accent-border);color:var(--accent,#9accfa)}

  /* Settings */
  .settings{max-width:600px}
  .settings section{background:var(--bg-elev-1,#181c20);border:1px solid var(--border,#1c2024);border-radius:10px;padding:6px 16px;margin-bottom:12px}
  .settings h3{font-size:11px;font-weight:600;color:var(--text-muted,#8c9198);text-transform:uppercase;letter-spacing:.8px;padding:10px 0 8px;border-bottom:1px solid var(--border,#1c2024)}
  .setting{display:flex;align-items:center;justify-content:space-between;padding:10px 0;border-bottom:1px solid var(--bg-elev-2,#1c2024)}
  .setting:last-child{border-bottom:none}
  .setting label{font-size:13px;color:var(--text-dim,#c2c7cf)}
  .setting input,.setting select{width:240px;padding:5px 10px;background:var(--bg-base,#101418);border:1px solid var(--border,#1c2024);color:var(--text,#e0e2e8);border-radius:6px;font-size:12px;outline:none;font-family:inherit;transition:border-color .12s}
  .setting input:focus,.setting select:focus{border-color:var(--accent-border)}
  .input-with-suffix{display:flex;align-items:center;gap:8px}
  .input-with-suffix input{width:90px}
  .input-with-suffix span{font-size:11px;color:var(--text-muted,#8c9198)}
  .hint{font-size:11px;color:var(--text-muted,#8c9198);padding:8px 0 12px;line-height:1.6}
  .hint code{background:var(--bg-base,#101418);padding:2px 6px;border-radius:4px;font-family:monospace;font-size:10px;color:var(--accent,#9accfa)}
  .save{padding:8px 18px;background:var(--accent-bg);border:1px solid var(--accent-border);color:var(--accent,#9accfa);border-radius:6px;font-size:12px;font-weight:500;cursor:pointer}
  .save:hover{background:rgba(154,204,250,.18)}
</style>
