<script setup lang="ts">
import { computed, ref, watch, onMounted, onUnmounted } from 'vue'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import MapModal from './components/MapModal.vue'
import GpxPreviewModal from './components/GpxPreviewModal.vue'

// ── 型別 ───────────────────────────────────────────────────────────────────

interface AppConfig {
  token: string; folder: string; event_id: string; location: string
  longitude: number; latitude: number; gpx_file: string; gpx_time_offset: number
  gpx_fallback_mode: string; gpx_max_gap: number; gpx_preview_count: number
  concurrency: number; timeout: number; auto_mode: boolean
}
interface EventInfo { id: string; name: string; date: string }
interface LogEntry { message: string; level: string; ts: number }
interface PreviewFile {
  abs_path: string; file_name: string; file_size: number; mtime_ns: number
  capture_time_minutes: number | null
}
type PhotoUploadStatus = 'pending' | 'uploading' | 'success' | 'duplicate' | 'failed'
type FeaturedSyncStatus = 'idle' | 'syncing' | 'synced' | 'error'
interface PreviewPhoto extends PreviewFile {
  previewUrl: string
  photoId: string | null
  status: PhotoUploadStatus
  isFeatured: boolean
  featuredSync: FeaturedSyncStatus
  thumbnailError: boolean
  message: string
}
interface UploadResultEvent {
  file_name: string; abs_path: string; success: boolean; duplicate: boolean
  photo_id: string | null; message: string
}
interface PreviewContext {
  folder: string
  eventId: string
}
export interface GpxPreviewRow {
  file_name: string; abs_path: string; capture_time_utc: string | null
  source: string; coord: string | null
  lat: number | null; lon: number | null
  note: string
}

// ── 狀態 ───────────────────────────────────────────────────────────────────

const isPreviewWindow = new URLSearchParams(window.location.search).get('window') === 'preview'

const loginStatus = ref<'none' | 'pending' | 'ok' | 'fail'>('none')
const loginLabel = ref('未登入')
const userRole = ref('')
const userName = ref('')

const folder = ref('')
const selectedEventId = ref('')
const location = ref('')
const longitude = ref(121.0)
const latitude = ref(25.0)
const gpxFile = ref('')
const gpxTimeOffset = ref(0)
const gpxFallbackMode = ref('manual')
const gpxMaxGap = ref(300)
const gpxPreviewCount = ref(10)
const concurrency = ref(20)  // 自動模式起始值；手動模式使用者自設
const autoMode = ref(true)  // true = 自動調整（預設），false = 手動
// 自動調整用的滑動窗口
const autoWindow = ref<boolean[]>([])
const AUTO_WINDOW_SIZE = 20
const AUTO_MAX_CONCURRENCY = 20
const timeout = ref(120)

const token = ref('')
const events = ref<EventInfo[]>([])
const savedEventId = ref('')  // fallback for manual input

const logs = ref<LogEntry[]>([])
const progress = ref(0)
const progressLabel = ref('準備就緒')
const isUploading = ref(false)
const failedCount = ref(0)
const previewPhotos = ref<PreviewPhoto[]>([])
const previewLoading = ref(false)
const previewPage = ref(1)
const previewPageInput = ref(1)
const captureTimeFilter = ref<'all' | 'with-time' | 'without-time'>('all')
const captureTimeFrom = ref('')
const captureTimeTo = ref('')
const captureTimesLoading = ref(false)
const captureTimesLoaded = ref(false)
const captureTimesError = ref('')
const existingFeaturedIds = ref<string[]>([])
const existingFeaturedLoading = ref(false)
const featuredTouchedPaths = new Set<string>()
const featureSyncQueues = new Map<string, Promise<void>>()
let previewRequestId = 0
const PREVIEW_PAGE_SIZE = 120

const currentEventId = computed(() => selectedEventId.value || savedEventId.value)
const isOrganizationTarget = computed(() => currentEventId.value.startsWith('org_'))
const featuredLimit = computed(() => isOrganizationTarget.value ? null : 10)
const featuredCount = computed(() => {
  const ids = new Set(existingFeaturedIds.value)
  let pendingCount = 0
  for (const photo of previewPhotos.value) {
    if (!photo.isFeatured) continue
    if (photo.photoId) ids.add(photo.photoId)
    else pendingCount += 1
  }
  return ids.size + pendingCount
})
const captureTimeFilterActive = computed(() => (
  captureTimeFilter.value !== 'all' || Boolean(captureTimeFrom.value) || Boolean(captureTimeTo.value)
))
const filteredPreviewPhotos = computed(() => previewPhotos.value.filter(photo => {
  if (captureTimeFilterActive.value && !captureTimesLoaded.value) return false
  if (captureTimeFilter.value === 'with-time' && photo.capture_time_minutes === null) return false
  if (captureTimeFilter.value === 'without-time' && photo.capture_time_minutes !== null) return false
  if (photo.capture_time_minutes === null) return !captureTimeFrom.value && !captureTimeTo.value

  const minutes = photo.capture_time_minutes
  const from = parseTimeInputMinutes(captureTimeFrom.value)
  const to = parseTimeInputMinutes(captureTimeTo.value)
  if (from !== null && minutes < from) return false
  if (to !== null && minutes > to) return false
  return true
}))
const previewPageCount = computed(() => Math.max(1, Math.ceil(filteredPreviewPhotos.value.length / PREVIEW_PAGE_SIZE)))
const visiblePreviewPhotos = computed(() => {
  const start = (previewPage.value - 1) * PREVIEW_PAGE_SIZE
  return filteredPreviewPhotos.value.slice(start, start + PREVIEW_PAGE_SIZE)
})

watch([captureTimeFilter, captureTimeFrom, captureTimeTo], () => {
  previewPage.value = 1
  previewPageInput.value = 1
  if (captureTimeFilterActive.value) void loadCaptureTimes()
})

const showMap = ref(false)
const showGpxPreview = ref(false)
const gpxPreviewRows = ref<GpxPreviewRow[]>([])
const gpxPreviewLoading = ref(false)

const logPanel = ref<HTMLElement | null>(null)

// ── 事件監聽 ────────────────────────────────────────────────────────────────

let unlisteners: UnlistenFn[] = []

async function checkForUpdates() {
  try {
    const update = await check()
    if (update?.available) {
      const yes = window.confirm(
        `有新版本 ${update.version} 可以更新！\n\n${update.body ?? ''}\n\n是否立即下載並安裝？`
      )
      if (yes) {
        await update.downloadAndInstall()
        await relaunch()
      }
    }
  } catch {
    // 靜默失敗，不影響正常使用
  }
}

async function notifyPreviewContext() {
  if (!isPreviewWindow) {
    await emit<PreviewContext>('preview://context', {
      folder: folder.value,
      eventId: currentEventId.value,
    }).catch(() => {})
  }
}

async function openPreviewWindow() {
  await saveConfig()

  const existing = await WebviewWindow.getByLabel('photo-preview')
  if (existing) {
    await existing.show()
    await existing.setFocus()
    await notifyPreviewContext()
    return
  }

  const preview = new WebviewWindow('photo-preview', {
    url: '/?window=preview',
    title: '相片預覽 · 運動拍檔',
    width: 1200,
    height: 860,
    minWidth: 900,
    minHeight: 650,
    resizable: true,
    center: true,
  })
  preview.once('tauri://error', ({ payload }) => {
    addLog(`開啟相片預覽失敗：${payload}`, 'error')
  })
}

onMounted(async () => {
  const cfg = await invoke<AppConfig>('cmd_get_config')
  applyConfig(cfg)

  if (!isPreviewWindow) checkForUpdates()

  if (isPreviewWindow) {
    unlisteners.push(await listen<PreviewContext>('preview://context', ({ payload }) => {
      folder.value = payload.folder
      selectedEventId.value = payload.eventId
      savedEventId.value = payload.eventId
      void scanPreview()
    }))
  }

  unlisteners.push(
    await listen<string>('auth://token', ({ payload }) => {
      doVerifyToken(payload)
    }),
    await listen<{ message: string; level: string; auto_result?: 'success' | 'failure' | 'duplicate' }>('upload://log', ({ payload }) => {
      logs.value.push({ ...payload, ts: Date.now() })
      if (payload.auto_result === 'success') recordAutoResult(true)
      else if (payload.auto_result === 'failure') recordAutoResult(false)
      setTimeout(() => {
        if (logPanel.value) logPanel.value.scrollTop = logPanel.value.scrollHeight
      }, 0)
    }),
    await listen<{ current: number; total: number; file_name: string; abs_path: string }>('upload://progress', ({ payload }) => {
      isUploading.value = true
      progress.value = payload.total > 0 ? Math.round(payload.current / payload.total * 100) : 0
      progressLabel.value = `上傳中：${payload.current} / ${payload.total}`
      const photo = previewPhotos.value.find(item => item.abs_path === payload.abs_path)
      if (photo && photo.status !== 'success' && photo.status !== 'duplicate') {
        photo.status = 'uploading'
      }
    }),
    await listen<UploadResultEvent>('upload://result', ({ payload }) => {
      handleUploadResult(payload)
    }),
    await listen<{ success: number; failed: number }>('upload://finished', ({ payload }) => {
      isUploading.value = false
      progress.value = 100
      progressLabel.value = `完成｜成功 ${payload.success}，失敗 ${payload.failed}`
      void refreshFailedCount()
    }),
  )
})

onUnmounted(() => unlisteners.forEach(u => u()))

// ── 設定 ───────────────────────────────────────────────────────────────────

function applyConfig(cfg: AppConfig) {
  token.value = cfg.token
  folder.value = cfg.folder
  savedEventId.value = cfg.event_id
  location.value = cfg.location
  longitude.value = cfg.longitude
  latitude.value = cfg.latitude
  gpxFile.value = cfg.gpx_file
  gpxTimeOffset.value = cfg.gpx_time_offset
  gpxFallbackMode.value = cfg.gpx_fallback_mode
  gpxMaxGap.value = cfg.gpx_max_gap
  gpxPreviewCount.value = cfg.gpx_preview_count
  autoMode.value = cfg.auto_mode ?? true
  const savedConcurrency = normalizeConcurrency(cfg.concurrency)
  concurrency.value = autoMode.value
    ? Math.min(savedConcurrency, AUTO_MAX_CONCURRENCY)
    : savedConcurrency
  timeout.value = cfg.timeout
  if (cfg.token) doVerifyToken(cfg.token)
}

async function saveConfig() {
  await invoke('cmd_save_config', {
    cfg: {
      token: token.value, folder: folder.value,
      event_id: selectedEventId.value || savedEventId.value,
      location: location.value, longitude: longitude.value, latitude: latitude.value,
      gpx_file: gpxFile.value, gpx_time_offset: gpxTimeOffset.value,
      gpx_fallback_mode: gpxFallbackMode.value, gpx_max_gap: gpxMaxGap.value,
      gpx_preview_count: gpxPreviewCount.value, concurrency: concurrency.value,
      timeout: timeout.value, auto_mode: autoMode.value,
    }
  }).catch(() => {})
}

// ── 登入 ───────────────────────────────────────────────────────────────────

async function doLogin() {
  loginStatus.value = 'pending'
  loginLabel.value = '等待授權...'
  await invoke('cmd_start_auth_server').catch(() => {
    loginStatus.value = 'fail'
    loginLabel.value = '啟動失敗'
  })
}

async function doVerifyToken(t: string) {
  try {
    const user = await invoke<{ role?: string; name?: string; email?: string }>('cmd_verify_token', { token: t })
    token.value = t
    loginStatus.value = 'ok'
    userRole.value = user.role ?? 'user'
    userName.value = user.email ?? user.name ?? ''
    loginLabel.value = userName.value ? `已登入：${userName.value}` : `已登入`
    await loadEvents()
    await saveConfig()
  } catch {
    loginStatus.value = 'fail'
    loginLabel.value = '登入失敗或 Token 過期'
    token.value = ''
    userName.value = ''
  }
}

async function loadEvents() {
  if (!token.value) return
  try {
    events.value = await invoke<EventInfo[]>('cmd_list_events', { token: token.value })
    if (!selectedEventId.value && savedEventId.value) {
      const match = events.value.find(e => e.id === savedEventId.value)
      selectedEventId.value = match ? match.id : (events.value[0]?.id ?? '')
    }
    await refreshFailedCount()
    if (isPreviewWindow) await scanPreview()
    else await notifyPreviewContext()
  } catch {}
}

async function refreshFailedCount() {
  const eid = currentEventId.value
  if (!eid) {
    failedCount.value = 0
    return
  }
  failedCount.value = await invoke<number>('cmd_get_failed_count', { eventId: eid }).catch(() => 0)
}

// ── 資料夾 / GPX 選擇 ───────────────────────────────────────────────────────

async function browseFolder() {
  const selected = await openDialog({ directory: true, multiple: false })
  if (typeof selected === 'string') {
    folder.value = selected
    await saveConfig()
    await notifyPreviewContext()
  }
}

async function onEventChange() {
  await refreshFailedCount()
  if (isPreviewWindow) await scanPreview()
  else {
    await saveConfig()
    await notifyPreviewContext()
  }
}

async function scanPreview() {
  if (!folder.value) {
    previewPhotos.value = []
    existingFeaturedIds.value = []
    existingFeaturedLoading.value = false
    captureTimesLoaded.value = false
    captureTimesError.value = ''
    previewPage.value = 1
    previewPageInput.value = 1
    return
  }

  const requestId = ++previewRequestId
  existingFeaturedLoading.value = false
  captureTimesLoaded.value = false
  captureTimesError.value = ''
  previewLoading.value = true
  try {
    const files = await invoke<PreviewFile[]>('cmd_preview_photos', { folder: folder.value })
    if (requestId !== previewRequestId) return
    existingFeaturedIds.value = []
    featuredTouchedPaths.clear()
    previewPage.value = 1
    previewPageInput.value = 1
    previewPhotos.value = files.map(file => ({
      ...file,
      previewUrl: convertFileSrc(file.abs_path),
      photoId: null,
      status: 'pending',
      isFeatured: false,
      featuredSync: 'idle',
      thumbnailError: false,
      message: '',
    }))
    if (captureTimeFilterActive.value) void loadCaptureTimes()
  } catch (e) {
    previewPhotos.value = []
    addLog(`相片預覽掃描失敗：${e}`, 'error')
    return
  } finally {
    if (requestId === previewRequestId) previewLoading.value = false
  }

  const eventId = currentEventId.value
  if (requestId !== previewRequestId || !eventId || !token.value) return

  existingFeaturedLoading.value = true
  try {
    const featuredIds = await invoke<string[]>('cmd_list_featured_photo_ids', {
      token: token.value,
      eventId,
    }).catch(() => [])
    if (requestId === previewRequestId) existingFeaturedIds.value = featuredIds
  } finally {
    if (requestId === previewRequestId) existingFeaturedLoading.value = false
  }
}

async function loadCaptureTimes() {
  if (!captureTimeFilterActive.value || captureTimesLoaded.value || captureTimesLoading.value || previewPhotos.value.length === 0) return

  captureTimesLoading.value = true
  captureTimesError.value = ''
  try {
    const captureTimes = await invoke<Record<string, number | null>>('cmd_preview_capture_times', {
      paths: previewPhotos.value.map(photo => photo.abs_path),
    })
    for (const photo of previewPhotos.value) {
        photo.capture_time_minutes = captureTimes[photo.abs_path] ?? null
    }
    captureTimesLoaded.value = true
  } catch (e) {
    captureTimesError.value = '拍攝時間讀取失敗，請重新嘗試。'
    addLog(`拍攝時間讀取失敗：${e}`, 'error')
  } finally {
    captureTimesLoading.value = false
  }
}

function updateFeaturedId(photoId: string, isFeatured: boolean) {
  const ids = new Set(existingFeaturedIds.value)
  if (isFeatured) ids.add(photoId)
  else ids.delete(photoId)
  existingFeaturedIds.value = [...ids]
}

function toggleFeatured(photo: PreviewPhoto) {
  if (!currentEventId.value) {
    alert('請先選擇活動')
    return
  }
  if (!isOrganizationTarget.value && existingFeaturedLoading.value) {
    alert('正在讀取目前精選狀態，請稍候再選擇')
    return
  }

  const nextValue = !photo.isFeatured
  if (nextValue && featuredLimit.value !== null && featuredCount.value >= featuredLimit.value) {
    alert(`每個活動最多只能精選 ${featuredLimit.value} 張照片`)
    return
  }

  photo.isFeatured = nextValue
  photo.featuredSync = 'idle'
  featuredTouchedPaths.add(photo.abs_path)
  if (photo.photoId) {
    updateFeaturedId(photo.photoId, nextValue)
    queueFeaturedSync(photo)
  }
}

function queueFeaturedSync(photo: PreviewPhoto) {
  if (!photo.photoId || !currentEventId.value || !token.value) return

  const path = photo.abs_path
  const photoId = photo.photoId
  const desiredValue = photo.isFeatured
  const previous = featureSyncQueues.get(path) ?? Promise.resolve()
  const next = previous
    .catch(() => {})
    .then(async () => {
      if (photo.photoId !== photoId || photo.isFeatured !== desiredValue) return
      photo.featuredSync = 'syncing'
      await invoke('cmd_set_photo_featured', {
        token: token.value,
        eventId: currentEventId.value,
        photoId,
        isFeatured: desiredValue,
        timeoutSecs: Math.min(Math.max(timeout.value, 10), 30),
      })
      if (photo.isFeatured === desiredValue) photo.featuredSync = 'synced'
    })
    .catch((e: unknown) => {
      if (photo.isFeatured === desiredValue) {
        photo.featuredSync = 'error'
        addLog(`精選同步失敗：${photo.file_name} - ${e}`, 'error')
      }
    })

  featureSyncQueues.set(path, next)
}

function handleUploadResult(payload: UploadResultEvent) {
  const photo = previewPhotos.value.find(item => item.abs_path === payload.abs_path)
  if (!photo) return

  photo.status = payload.success
    ? (payload.duplicate ? 'duplicate' : 'success')
    : 'failed'
  photo.message = payload.message
  if (payload.photo_id) {
    photo.photoId = payload.photo_id

    const wasTouched = featuredTouchedPaths.has(photo.abs_path)
    if (!wasTouched && existingFeaturedIds.value.includes(payload.photo_id)) {
      photo.isFeatured = true
    }
    updateFeaturedId(payload.photo_id, photo.isFeatured)
    if (photo.isFeatured || wasTouched) queueFeaturedSync(photo)
  }
}

function onPreviewImageError(photo: PreviewPhoto) {
  photo.thumbnailError = true
}

function previewStatusLabel(photo: PreviewPhoto) {
  if (photo.featuredSync === 'syncing') return '精選同步中'
  if (photo.featuredSync === 'error') return '精選同步失敗'
  if (photo.status === 'uploading') return '上傳中'
  if (photo.status === 'success') return '已上傳'
  if (photo.status === 'duplicate') return '已存在'
  if (photo.status === 'failed') return '上傳失敗'
  return '待上傳'
}

function formatFileSize(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function parseTimeInputMinutes(value: string) {
  if (!value) return null
  const [hours, minutes] = value.split(':').map(Number)
  if (!Number.isInteger(hours) || !Number.isInteger(minutes)) return null
  return hours * 60 + minutes
}

function jumpPreviewPage() {
  const target = Number.isFinite(Number(previewPageInput.value))
    ? Math.floor(Number(previewPageInput.value))
    : 1
  previewPage.value = Math.min(previewPageCount.value, Math.max(1, target))
  previewPageInput.value = previewPage.value
}

function previousPreviewPage() {
  previewPageInput.value = previewPage.value - 1
  jumpPreviewPage()
}

function nextPreviewPage() {
  previewPageInput.value = previewPage.value + 1
  jumpPreviewPage()
}

function clearCaptureFilter() {
  captureTimeFilter.value = 'all'
  captureTimeFrom.value = ''
  captureTimeTo.value = ''
}

async function browseGpx() {
  const selected = await openDialog({
    multiple: false,
    filters: [{ name: 'GPX', extensions: ['gpx'] }]
  })
  if (typeof selected === 'string') gpxFile.value = selected
}

// ── 地圖選點 ─────────────────────────────────────────────────────────────────

function onMapConfirm(lat: number, lon: number) {
  latitude.value = lat
  longitude.value = lon
  showMap.value = false
  addLog(`已選擇座標：緯度 ${lat.toFixed(6)}, 經度 ${lon.toFixed(6)}`, 'info')
}

// ── GPX 預覽 ─────────────────────────────────────────────────────────────────

async function previewGpx() {
  if (!folder.value || !gpxFile.value) { alert('請先選擇相片資料夾與 GPX 檔案'); return }
  gpxPreviewLoading.value = true
  try {
    gpxPreviewRows.value = await invoke<GpxPreviewRow[]>('cmd_preview_gpx', {
      params: {
        folder: folder.value, gpx_file: gpxFile.value,
        time_offset_secs: gpxTimeOffset.value,
        fallback_lat: latitude.value || null,
        fallback_lon: longitude.value || null,
        fallback_mode: gpxFallbackMode.value,
        max_gap_secs: gpxMaxGap.value,
        sample_count: gpxPreviewCount.value,
      }
    })
    showGpxPreview.value = true
  } catch (e) {
    alert(`GPX 預覽失敗：${e}`)
  } finally {
    gpxPreviewLoading.value = false
  }
}

// ── 並行數自動調整 ────────────────────────────────────────────────────────────

function normalizeConcurrency(value: number): number {
  const n = Number.isFinite(value) ? Math.floor(value) : 1
  return Math.max(1, n)
}

// 手動修改並行數 → 切換為手動模式
function onConcurrencyInput() {
  autoMode.value = false
  concurrency.value = normalizeConcurrency(concurrency.value)
  invoke('cmd_set_concurrency', { n: concurrency.value }).catch(() => {})
}

function onConcurrencyChange() {
  void saveConfig()
}

function toggleAutoMode() {
  autoMode.value = !autoMode.value
  autoWindow.value = []
  if (autoMode.value) {
    concurrency.value = Math.min(normalizeConcurrency(concurrency.value), AUTO_MAX_CONCURRENCY)
    invoke('cmd_set_auto_concurrency', { n: concurrency.value }).catch(() => {})
  } else {
    concurrency.value = normalizeConcurrency(concurrency.value)
    invoke('cmd_set_concurrency', { n: concurrency.value }).catch(() => {})
  }
  void saveConfig()
}

// 每次上傳結果（success/error）都會呼叫此函式
function recordAutoResult(success: boolean) {
  if (!autoMode.value || !isUploading.value) return
  autoWindow.value.push(success)
  if (autoWindow.value.length > AUTO_WINDOW_SIZE)
    autoWindow.value.shift()
  if (autoWindow.value.length < AUTO_WINDOW_SIZE) return

  const successRate = autoWindow.value.filter(v => v).length / AUTO_WINDOW_SIZE
  let newVal = concurrency.value
  if (successRate >= 0.95) {
    // 成功率高 → 增加 2
    newVal = Math.min(concurrency.value + 2, AUTO_MAX_CONCURRENCY)
  } else if (successRate < 0.8) {
    // 失敗率過高 → 減少 30%
    newVal = Math.max(1, Math.floor(concurrency.value * 0.7))
  }
  autoWindow.value = [] // 每 20 個結果只做一次決策
  if (newVal !== concurrency.value) {
    concurrency.value = newVal
    invoke('cmd_set_auto_concurrency', { n: newVal }).catch(() => {})
    addLog(`自動調整並行數 → ${newVal}（成功率 ${Math.round(successRate * 100)}%）`, 'info')
  }
}

// ── 上傳 ───────────────────────────────────────────────────────────────────

function addLog(message: string, level = 'info') {
  logs.value.push({ message, level, ts: Date.now() })
}

function validateInputs(): boolean {
  if (!token.value) { alert('請先登入'); return false }
  if (!folder.value) { alert('請選擇相片資料夾'); return false }
  if (!currentEventId.value) { alert('請選擇活動'); return false }
  if (!location.value.trim()) { alert('請輸入拍攝地點'); return false }
  return true
}

function validateRetryInputs(): boolean {
  if (!token.value) { alert('請先登入'); return false }
  if (!currentEventId.value) { alert('請選擇活動'); return false }
  return true
}

function buildUploadParams() {
  return {
    token: token.value,
    event_id: currentEventId.value,
    location: location.value,
    folder: folder.value,
    longitude: longitude.value || null,
    latitude: latitude.value || null,
    gpx_file: gpxFile.value || null,
    gpx_time_offset: gpxTimeOffset.value,
    gpx_fallback_mode: gpxFallbackMode.value,
    gpx_max_gap: gpxMaxGap.value,
    auto_mode: autoMode.value,
    concurrency: concurrency.value,
    timeout_secs: timeout.value,
  }
}

async function startUpload() {
  if (!validateInputs()) return
  await saveConfig()
  isUploading.value = true
  progress.value = 0
  progressLabel.value = '準備上傳...'
  autoWindow.value = [] // 重置自動調整窗口

  await invoke('cmd_start_upload', {
    params: buildUploadParams(),
  }).catch((e: unknown) => {
    addLog(`啟動失敗：${e}`, 'error')
    isUploading.value = false
  })
}

async function retryFailed() {
  if (isUploading.value || failedCount.value === 0 || !validateRetryInputs()) return
  await saveConfig()
  isUploading.value = true
  progress.value = 0
  progressLabel.value = `準備重試 ${failedCount.value} 張...`
  autoWindow.value = []

  await invoke('cmd_retry_failed', {
    params: buildUploadParams(),
  }).catch((e: unknown) => {
    addLog(`重試啟動失敗：${e}`, 'error')
    isUploading.value = false
  })
}

async function stopUpload() {
  await invoke('cmd_stop_upload').catch(() => {})
  isUploading.value = false
  progressLabel.value = '已停止'
}

async function clearLog() {
  const eid = currentEventId.value
  if (!eid || !confirm(`確定清除活動 ${eid} 的上傳紀錄？`)) return
  try {
    await invoke('cmd_clear_history', { eventId: eid })
    logs.value = []
    progress.value = 0
    progressLabel.value = '準備就緒'
    failedCount.value = 0
    addLog(`已清除活動 ${eid} 的上傳紀錄`, 'info')
  } catch (e: unknown) {
    addLog(`清除紀錄失敗：${e}`, 'error')
  }
}

function logClass(level: string) {
  return { success: 'log-success', error: 'log-error', warn: 'log-warn' }[level] ?? 'log-info'
}

function eventLabel(ev: EventInfo) { return `${ev.id} (${ev.name} - ${ev.date})` }
</script>

<template>
  <div class="app">
    <header class="header">
      <img src="/logo.svg" class="header-logo" alt="運動拍檔 RaceShot" />
      <span class="header-divider"></span>
      <span class="header-tagline">PHOTO UPLOADER</span>
      <div style="flex:1"></div>
      <template v-if="isPreviewWindow">
        <span class="header-preview-label">PHOTO PREVIEW · 相片精選</span>
      </template>
      <template v-else>
        <span :class="['status', `status--${loginStatus}`]">{{ loginLabel }}</span>
        <button class="btn btn--primary btn--sm" @click="doLogin" :disabled="isUploading">網頁登入</button>
      </template>
    </header>

    <main v-if="isPreviewWindow" class="main preview-main">
      <section class="card photo-preview-card photo-preview-window-card">
        <div class="photo-preview-header">
          <div>
            <h2 class="section-title photo-preview-title">PHOTO PREVIEW · 本次相片</h2>
            <div class="photo-preview-summary">
              {{ filteredPreviewPhotos.length }} / {{ previewPhotos.length }} 張 · 已選精選 {{ featuredCount }}<span v-if="featuredLimit !== null"> / {{ featuredLimit }}</span>
              <span v-if="isOrganizationTarget"> · 主辦相簿</span>
              <span v-else> · 攝影師活動</span>
            </div>
          </div>
          <button class="btn btn--outline btn--sm" @click="scanPreview" :disabled="previewLoading">
            {{ previewLoading ? '掃描中…' : '重新掃描' }}
          </button>
        </div>
        <div class="photo-filter-row">
          <span class="sublabel">拍攝時間</span>
          <select class="input" v-model="captureTimeFilter">
            <option value="all">全部相片</option>
            <option value="with-time">有拍攝時間</option>
            <option value="without-time">沒有拍攝時間</option>
          </select>
          <input class="input" type="time" v-model="captureTimeFrom" :disabled="captureTimeFilter === 'without-time'" />
          <span class="sublabel">至</span>
          <input class="input" type="time" v-model="captureTimeTo" :disabled="captureTimeFilter === 'without-time'" />
          <button
            v-if="captureTimeFilter !== 'all' || captureTimeFrom || captureTimeTo"
            class="btn btn--clear"
            @click="clearCaptureFilter"
          >清除篩選</button>
        </div>
        <div v-if="isUploading" class="photo-preview-note">上傳進行中，可繼續選取精選照</div>

        <div v-if="previewLoading" class="photo-preview-empty">正在讀取相片清單…</div>
        <div v-else-if="captureTimesLoading" class="photo-preview-empty">正在讀取 EXIF 拍攝時間…</div>
        <div v-else-if="captureTimesError" class="photo-preview-empty">{{ captureTimesError }}</div>
        <div v-else-if="previewPhotos.length === 0" class="photo-preview-empty">此資料夾沒有支援的 JPG、JPEG 或 PNG 相片。</div>
        <div v-else-if="filteredPreviewPhotos.length === 0" class="photo-preview-empty">目前的拍攝時間篩選沒有相片。</div>
        <template v-else>
          <div class="photo-grid">
            <article v-for="photo in visiblePreviewPhotos" :key="photo.abs_path" class="photo-tile">
              <div class="photo-thumb">
                <img
                  v-if="!photo.thumbnailError"
                  :src="photo.previewUrl"
                  :alt="photo.file_name"
                  loading="lazy"
                  decoding="async"
                  @error="onPreviewImageError(photo)"
                />
                <div v-else class="photo-thumb-error">無法讀取</div>
                <button
                  class="photo-feature-button"
                  :class="{ 'photo-feature-button--active': photo.isFeatured }"
                  :disabled="!currentEventId"
                  :title="photo.isFeatured ? '取消精選' : '加入精選'"
                  @click.stop="toggleFeatured(photo)"
                >
                  ★
                </button>
                <span :class="['photo-status', `photo-status--${photo.status}`]">{{ previewStatusLabel(photo) }}</span>
              </div>
              <div class="photo-name" :title="photo.file_name">{{ photo.file_name }}</div>
              <div class="photo-size">{{ formatFileSize(photo.file_size) }}</div>
            </article>
          </div>
          <div class="photo-preview-footer">
            <span>第 {{ previewPage }} / {{ previewPageCount }} 頁</span>
            <div class="row gap8">
              <button class="btn btn--outline btn--sm" @click="previousPreviewPage" :disabled="previewPage <= 1">上一頁</button>
              <input
                class="input page-input"
                type="number"
                min="1"
                :max="previewPageCount"
                v-model.number="previewPageInput"
                @keyup.enter="jumpPreviewPage"
                aria-label="指定預覽分頁"
              />
              <button class="btn btn--outline btn--sm" @click="jumpPreviewPage">跳轉</button>
              <button class="btn btn--outline btn--sm" @click="nextPreviewPage" :disabled="previewPage >= previewPageCount">下一頁</button>
            </div>
          </div>
        </template>
      </section>
    </main>

    <main v-else class="main">

      <!-- 上傳參數 -->
      <section class="card">
        <h2 class="section-title">UPLOAD SETTINGS · 上傳參數</h2>

        <div class="field">
          <label class="field-label">相片資料夾</label>
          <div class="row">
            <input class="input flex1" :value="folder" readonly placeholder="點擊瀏覽選擇資料夾" />
            <button class="btn btn--outline" @click="browseFolder" :disabled="isUploading">瀏覽</button>
            <button class="btn btn--gold" @click="openPreviewWindow" :disabled="!folder">預覽與設定精選</button>
          </div>
        </div>

        <div class="field">
          <label class="field-label">活動選擇</label>
          <div class="row">
            <select class="input flex1" v-model="selectedEventId" :disabled="isUploading" @change="onEventChange">
              <option value="">— 請登入後更新活動列表 —</option>
              <option v-for="ev in events" :key="ev.id" :value="ev.id">{{ eventLabel(ev) }}</option>
            </select>
            <button class="btn btn--outline" @click="loadEvents" :disabled="!token || isUploading">更新活動</button>
          </div>
        </div>

        <!-- 拍攝地點 + 座標 合一排 -->
        <div class="field">
          <label class="field-label">拍攝地點 / 座標</label>
          <div class="row">
            <input class="input flex1" v-model="location" placeholder="例如：終點線" :disabled="isUploading" />
            <span class="sublabel">緯</span>
            <input class="input w90" type="number" v-model.number="latitude" step="0.000001" :disabled="isUploading" />
            <span class="sublabel">經</span>
            <input class="input w90" type="number" v-model.number="longitude" step="0.000001" :disabled="isUploading" />
            <button class="btn btn--outline-red" @click="showMap = true" :disabled="isUploading">地圖選點</button>
          </div>
        </div>

        <div class="field">
          <label class="field-label">GPX 軌跡</label>
          <div class="row">
            <input class="input flex1" :value="gpxFile" readonly placeholder="可選 · 選擇 .gpx 自動匹配座標" />
            <button v-if="gpxFile" class="btn btn--clear" @click="gpxFile = ''" :disabled="isUploading" title="清除 GPX">✕</button>
            <button class="btn btn--outline" @click="browseGpx" :disabled="isUploading">選擇 GPX</button>
            <button class="btn btn--gold" @click="previewGpx" :disabled="isUploading || gpxPreviewLoading">
              {{ gpxPreviewLoading ? '預覽中…' : '抽樣預覽' }}
            </button>
          </div>
        </div>

        <div class="field gpx-settings" v-if="gpxFile">
          <label class="field-label">GPX 設定</label>
          <div class="row wrap gap8">
            <span class="sublabel">時間偏移(秒)</span>
            <input class="input w80" type="number" v-model.number="gpxTimeOffset" :disabled="isUploading" />
            <span class="sublabel">匹配不到</span>
            <select class="input" v-model="gpxFallbackMode" :disabled="isUploading">
              <option value="manual">改用手動座標</option>
              <option value="empty">留空</option>
            </select>
            <span class="sublabel">最大間隔(秒)</span>
            <input class="input w80" type="number" v-model.number="gpxMaxGap" :disabled="isUploading" />
            <span class="sublabel">預覽張數</span>
            <input class="input w80" type="number" v-model.number="gpxPreviewCount" :disabled="isUploading" />
          </div>
        </div>

        <details class="advanced">
          <summary class="field-label">ADVANCED · 進階設定</summary>
          <div class="row wrap gap8 mt8">
            <span class="sublabel">並行數</span>
            <input class="input w80" type="number" v-model.number="concurrency" min="1"
              :disabled="isUploading && autoMode"
              @input="onConcurrencyInput" @change="onConcurrencyChange" />
            <button
              :class="['btn', 'btn--sm', autoMode ? 'btn--gold' : 'btn--outline']"
              @click="toggleAutoMode"
              :disabled="isUploading"
              :title="autoMode ? '自動模式：依網路成功率調整' : '手動模式：固定並行數'">
              {{ autoMode ? '⚡ 自動' : '手動' }}
            </button>
            <span class="sublabel">逾時(秒)</span>
            <input class="input w80" type="number" v-model.number="timeout" min="10" max="300" :disabled="isUploading" />
          </div>
        </details>
      </section>

      <!-- 進度 + 按鈕 (在日誌上方) -->
      <section class="card progress-card">
        <div class="progress-bar-wrap">
          <div class="progress-bar" :style="{ width: progress + '%' }"></div>
        </div>
        <p class="progress-label">{{ progressLabel }}</p>
        <div class="btn-row">
          <button class="btn btn--primary btn--lg" @click="startUpload" :disabled="isUploading">開始上傳</button>
          <button class="btn btn--gold btn--lg" @click="retryFailed" :disabled="isUploading || failedCount === 0 || !token">
            重試失敗 ({{ failedCount }})
          </button>
          <button class="btn btn--dark btn--lg" @click="stopUpload" :disabled="!isUploading">停止</button>
          <button class="btn btn--outline btn--lg" @click="clearLog" :disabled="isUploading">清除紀錄</button>
        </div>
      </section>

      <!-- 日誌 -->
      <section class="card card--dark log-card">
        <div class="log-header">
          <span class="section-title-dark">UPLOAD LOG · 上傳日誌</span>
        </div>
        <div class="log-panel" ref="logPanel">
          <div v-if="logs.length === 0" class="log-empty">日誌將在此顯示…</div>
          <div v-for="(entry, i) in logs" :key="i" :class="['log-line', logClass(entry.level)]">
            {{ entry.message }}
          </div>
        </div>
      </section>
    </main>

    <MapModal v-if="showMap" :init-lat="latitude" :init-lon="longitude" @confirm="onMapConfirm" @close="showMap = false" />
    <GpxPreviewModal v-if="showGpxPreview" :rows="gpxPreviewRows" @close="showGpxPreview = false" />
  </div>
</template>

<style>
@import url('https://fonts.googleapis.com/css2?family=LINE+Seed+Sans+TW:wght@400;700&display=swap');

/* ── 變數 ── */
:root {
  --red:      #B3262A;
  --red-dark: #8A1C1F;
  --red-light:#CC3A3E;
  --black:    #212121;
  --n90:      #333333;
  --n60:      #666260;
  --n40:      #9E9998;
  --n20:      #D8D5D0;
  --n05:      #F0EEEB;
  --gold:     #C9952A;
  --gold-dark:#8C6228;
  --white:    #FFFFFF;
}

* { box-sizing: border-box; margin: 0; padding: 0; }

body {
  font-family: 'LINE Seed Sans TW', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  background: var(--n05);
  color: var(--black);
  font-size: 14px;
  -webkit-font-smoothing: antialiased;
}

/* ── 版面 ── */
.app { display: flex; flex-direction: column; height: 100vh; overflow: hidden; }

.header {
  background: var(--black);
  padding: 0 12px;
  height: 46px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  border-bottom: 2px solid var(--red);
}
.header-logo { height: 26px; display: block; }
.header-divider { width: 1px; height: 18px; background: var(--n60); flex-shrink: 0; }
.header-tagline {
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.2em;
  color: var(--n40);
}
.header-preview-label {
  color: var(--white);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.12em;
}

.main {
  padding: 10px 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  overflow-y: auto;
  flex: 1;
}
.preview-main { padding: 14px; }

/* ── 卡片 ── */
.card {
  background: var(--white);
  border-radius: 6px;
  padding: 12px 16px;
  border: 1px solid var(--n20);
}
.card--dark {
  background: var(--black);
  border-color: var(--black);
}

/* ── 標題 ── */
.section-title {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.15em;
  color: var(--n60);
  text-transform: uppercase;
  margin-bottom: 10px;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--n20);
}
.section-title-dark {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.15em;
  color: var(--n40);
  text-transform: uppercase;
}
.log-header {
  display: flex;
  align-items: center;
  margin-bottom: 8px;
}

/* ── 欄位 ── */
.field { margin-bottom: 8px; }
.field:last-child { margin-bottom: 0; }
.field-label {
  display: block;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--n60);
  margin-bottom: 4px;
}
/* ACCOUNT 列使用 inline label，不佔獨立行 */
.label-inline {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--n60);
  white-space: nowrap;
  line-height: 1;
  align-self: center;
}
.row { display: flex; align-items: center; gap: 8px; }
.wrap { flex-wrap: wrap; }
.gap8 { gap: 8px; }
.mt8 { margin-top: 8px; }
.flex1 { flex: 1; min-width: 0; }
.sublabel { font-size: 12px; color: var(--n60); white-space: nowrap; }
.w80 { width: 80px; }
.w90 { width: 90px; }
.w120 { width: 120px; }

/* ── 輸入 ── */
.input {
  padding: 6px 10px;
  border: 1px solid var(--n20);
  border-radius: 4px;
  font-size: 13px;
  font-family: inherit;
  outline: none;
  background: var(--white);
  color: var(--black);
  transition: border-color .15s;
  height: 30px;
  box-sizing: border-box;
}
.input:focus { border-color: var(--red); }
.input:disabled { background: var(--n05); color: var(--n40); }
.input--full { width: 100%; }
select.input {
  cursor: pointer;
  -webkit-appearance: none;
  appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath fill='%239E9998' d='M0 0l5 6 5-6z'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 10px center;
  padding-right: 28px;
}

/* ── 按鈕 ── */
.btn {
  padding: 7px 14px;
  border: none;
  border-radius: 4px;
  font-size: 13px;
  font-family: inherit;
  font-weight: 700;
  cursor: pointer;
  white-space: nowrap;
  transition: background .15s, color .15s, opacity .15s;
  letter-spacing: 0.02em;
}
.btn:disabled { opacity: .4; cursor: not-allowed; }

.btn--primary { background: var(--red); color: var(--white); }
.btn--primary:hover:not(:disabled) { background: var(--red-dark); }

.btn--dark { background: var(--n90); color: var(--white); }
.btn--dark:hover:not(:disabled) { background: var(--black); }

.btn--gold { background: var(--gold); color: var(--white); }
.btn--gold:hover:not(:disabled) { background: var(--gold-dark); }

.btn--outline {
  background: transparent;
  color: var(--n90);
  border: 1px solid var(--n20);
}
.btn--outline:hover:not(:disabled) { background: var(--n05); border-color: var(--n40); }

.btn--clear {
  background: transparent;
  color: var(--n40);
  border: 1px solid var(--n20);
  padding: 6px 8px;
  font-size: 11px;
  line-height: 1;
  flex-shrink: 0;
}
.btn--clear:hover:not(:disabled) { color: var(--red); border-color: var(--red); }

.btn--outline-red {
  background: transparent;
  color: var(--red);
  border: 1px solid var(--red);
}
.btn--outline-red:hover:not(:disabled) { background: var(--red); color: var(--white); }

.btn--lg { padding: 11px 24px; font-size: 14px; }
.btn--sm { padding: 4px 10px; font-size: 12px; }

.btn--outline-header {
  background: transparent;
  color: var(--n40);
  border: 1px solid var(--n60);
}
.btn--outline-header:hover:not(:disabled) { border-color: var(--n40); color: var(--white); }

/* ── 狀態 ── */
.status {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.04em;
  padding: 3px 10px;
  border-radius: 2px;
}
.status--none    { color: var(--n40); background: var(--n05); }
.status--pending { color: var(--gold); background: #fdf6e3; }
.status--ok      { color: var(--red-dark); background: #fdeaea; border: 1px solid #f5c0c0; }
.status--fail    { color: var(--white); background: var(--red); }

/* ── 日誌 ── */
.log-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }
.log-panel {
  flex: 1;
  min-height: 80px;
  max-height: 160px;
  overflow-y: auto;
  font-family: 'Menlo', 'Consolas', monospace;
  font-size: 12px;
  line-height: 1.75;
}
.log-empty { color: var(--n60); }
.log-info    { color: #9E9998; }
.log-success { color: #6fcf97; }
.log-error   { color: #eb5757; }
.log-warn    { color: var(--gold); }

/* ── 相片預覽 ── */
.photo-preview-card { padding-bottom: 10px; }
.photo-preview-window-card { min-height: 100%; }
.photo-preview-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
.photo-preview-title { margin-bottom: 2px; border-bottom: 0; padding-bottom: 0; }
.photo-preview-summary { color: var(--n60); font-size: 12px; }
.photo-filter-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--n20);
}
.photo-filter-row .input { width: auto; min-width: 132px; }
.page-input { width: 58px !important; min-width: 58px !important; padding-left: 6px; padding-right: 6px; text-align: center; }
.photo-preview-empty {
  color: var(--n60);
  text-align: center;
  padding: 22px 8px 12px;
}
.photo-preview-note {
  margin-top: 10px;
  padding: 8px 10px;
  border-radius: 4px;
  background: #fdf6e3;
  color: var(--gold-dark);
  font-size: 12px;
}
.photo-grid {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: 12px;
  margin-top: 10px;
}
.photo-tile { min-width: 0; }
.photo-thumb {
  aspect-ratio: 3 / 2;
  background: var(--black);
  border: 1px solid var(--n20);
  border-radius: 4px;
  overflow: hidden;
  position: relative;
}
.photo-thumb img { width: 100%; height: 100%; display: block; object-fit: contain; }
.photo-thumb-error {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--n40);
  font-size: 11px;
}
.photo-feature-button {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 25px;
  height: 25px;
  border: 0;
  border-radius: 50%;
  background: rgba(33,33,33,.7);
  color: var(--white);
  cursor: pointer;
  font-size: 16px;
  line-height: 1;
}
.photo-feature-button:hover:not(:disabled) { background: var(--gold-dark); }
.photo-feature-button--active { background: var(--gold); color: var(--white); }
.photo-feature-button:disabled { opacity: .45; cursor: not-allowed; }
.photo-status {
  position: absolute;
  left: 4px;
  bottom: 4px;
  padding: 2px 4px;
  border-radius: 2px;
  background: rgba(33,33,33,.75);
  color: var(--white);
  font-size: 10px;
  line-height: 1.2;
}
.photo-status--success { background: rgba(39,120,72,.85); }
.photo-status--duplicate { background: rgba(140,98,40,.9); }
.photo-status--failed { background: rgba(179,38,42,.9); }
.photo-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--n90);
  font-size: 11px;
  margin-top: 4px;
}
.photo-size { color: var(--n40); font-size: 10px; }
.photo-preview-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  color: var(--n60);
  font-size: 11px;
  margin-top: 10px;
}

/* ── 進度 ── */
.progress-card { padding: 10px 16px; }
.progress-bar-wrap {
  height: 4px;
  background: var(--n20);
  border-radius: 2px;
  overflow: hidden;
}
.progress-bar {
  height: 100%;
  background: var(--red);
  transition: width .3s ease;
}
.progress-label {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.08em;
  color: var(--n60);
  margin-top: 6px;
  text-align: center;
  text-transform: uppercase;
}

/* ── 操作按鈕列 ── */
.btn-row { display: flex; gap: 10px; margin-top: 10px; }
.btn-row .btn { flex: 1; min-width: 0; }

@media (max-width: 1050px) {
  .photo-grid { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}

@media (max-width: 760px) {
  .photo-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }
}

/* ── 進階設定 ── */
.advanced { margin-top: 8px; }
.advanced summary {
  cursor: pointer;
  list-style: none;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.advanced summary::before { content: '▸'; color: var(--n40); font-size: 10px; }
.advanced[open] summary::before { content: '▾'; }
.gpx-settings {
  background: var(--n05);
  border-radius: 4px;
  padding: 10px 12px;
  margin-top: -4px;
}

/* ── Modal ── */
.modal-overlay {
  position: fixed; inset: 0;
  background: rgba(33,33,33,.6);
  display: flex; align-items: center; justify-content: center;
  z-index: 1000;
}
.modal {
  background: var(--white);
  border-radius: 6px;
  box-shadow: 0 16px 48px rgba(0,0,0,.3);
  display: flex; flex-direction: column;
  overflow: hidden;
}
.modal-header {
  padding: 14px 18px;
  background: var(--black);
  color: var(--white);
  display: flex; align-items: center; justify-content: space-between;
  border-bottom: 2px solid var(--red);
}
.modal-header h3 {
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}
.modal-close {
  background: none; border: none;
  color: var(--n40); font-size: 20px;
  cursor: pointer; line-height: 1;
  transition: color .15s;
}
.modal-close:hover { color: var(--white); }
.modal-footer {
  padding: 12px 18px;
  display: flex; gap: 8px; justify-content: flex-end;
  border-top: 1px solid var(--n20);
}
</style>
