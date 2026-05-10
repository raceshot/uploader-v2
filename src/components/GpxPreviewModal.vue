<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'
import { convertFileSrc } from '@tauri-apps/api/core'

export interface GpxPreviewRow {
  file_name: string
  abs_path: string
  capture_time_utc: string | null
  source: string
  coord: string | null
  lat: number | null
  lon: number | null
  note: string
}

const props = defineProps<{ rows: GpxPreviewRow[] }>()
defineEmits<{ close: [] }>()

const mapEl = ref<HTMLElement | null>(null)
let map: L.Map | null = null

const mappable = computed(() => props.rows.filter(r => r.lat != null && r.lon != null))

const stats = computed(() => ({
  gpx: props.rows.filter(r => r.source === 'gpx').length,
  fallback: props.rows.filter(r => r.source === 'manual-fallback').length,
  unmatched: props.rows.filter(r => r.source === 'unmatched').length,
}))

function markerColor(source: string): string {
  if (source === 'gpx') return '#22863a'
  if (source === 'manual-fallback') return '#b08800'
  return '#b3262a'
}

function makeIcon(source: string) {
  const color = markerColor(source)
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="36" viewBox="0 0 24 36">
    <path d="M12 0C5.4 0 0 5.4 0 12c0 9 12 24 12 24s12-15 12-24C24 5.4 18.6 0 12 0z" fill="${color}" stroke="white" stroke-width="1.5"/>
    <circle cx="12" cy="12" r="5" fill="white"/>
  </svg>`
  return L.divIcon({
    html: svg,
    iconSize: [24, 36],
    iconAnchor: [12, 36],
    popupAnchor: [0, -36],
    className: '',
  })
}

function sourceLabel(source: string): string {
  if (source === 'gpx') return 'GPX 匹配'
  if (source === 'manual-fallback') return '手動座標'
  return '未匹配'
}

function sourceBadgeClass(source: string): string {
  if (source === 'gpx') return 'badge badge--green'
  if (source === 'manual-fallback') return 'badge badge--yellow'
  return 'badge badge--red'
}

onMounted(() => {
  if (!mapEl.value || mappable.value.length === 0) return

  const first = mappable.value[0]
  map = L.map(mapEl.value).setView([first.lat!, first.lon!], 13)

  L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
    attribution: '© OpenStreetMap contributors',
    maxZoom: 19,
  }).addTo(map)

  const bounds: L.LatLngTuple[] = []

  mappable.value.forEach((row, idx) => {
    const latlng: L.LatLngTuple = [row.lat!, row.lon!]
    bounds.push(latlng)

    const imgSrc = convertFileSrc(row.abs_path)
    const marker = L.marker(latlng, { icon: makeIcon(row.source) }).addTo(map!)
    marker.bindPopup(`
      <div style="font-size:12px;line-height:1.6;width:220px">
        <img src="${imgSrc}"
          style="width:100%;height:140px;object-fit:cover;border-radius:4px;display:block;margin-bottom:6px;background:#eee"
          loading="lazy" />
        <b style="font-size:12px">#${idx + 1} ${row.file_name}</b><br>
        <span style="color:${markerColor(row.source)};font-size:11px">${sourceLabel(row.source)}</span>
        ${row.capture_time_utc ? `<br><span style="color:#555;font-size:11px">🕐 ${row.capture_time_utc}</span>` : ''}
        <br><span style="font-family:monospace;font-size:11px">📍 ${row.coord ?? '—'}</span>
        ${row.note ? `<br><span style="color:#888;font-size:11px">${row.note}</span>` : ''}
      </div>
    `, { maxWidth: 240 })
  })

  if (bounds.length > 1) {
    map.fitBounds(bounds, { padding: [40, 40] })
  } else if (bounds.length === 1) {
    map.setView(bounds[0], 15)
  }
})

onUnmounted(() => { map?.remove() })
</script>

<template>
  <div class="modal-overlay" @click.self="$emit('close')">
    <div class="modal gpx-modal">
      <div class="modal-header">
        <h3>GPX 抽樣預覽</h3>
        <div class="header-stats">
          <span class="badge badge--green">GPX {{ stats.gpx }}</span>
          <span class="badge badge--yellow">手動 {{ stats.fallback }}</span>
          <span class="badge badge--red">未匹配 {{ stats.unmatched }}</span>
        </div>
        <button class="modal-close" @click="$emit('close')">×</button>
      </div>

      <!-- 地圖 -->
      <div class="map-wrap">
        <div v-if="mappable.length === 0" class="map-empty">
          無可顯示的座標（所有樣本均未匹配）
        </div>
        <div v-else ref="mapEl" class="map-el"></div>
        <div class="map-legend">
          <span><i class="dot dot--green"></i>GPX 匹配</span>
          <span><i class="dot dot--yellow"></i>手動座標</span>
          <span><i class="dot dot--red"></i>未匹配</span>
        </div>
      </div>

      <!-- 表格 -->
      <div class="preview-body">
        <table class="preview-table">
          <thead>
            <tr>
              <th>#</th>
              <th>檔名</th>
              <th>來源</th>
              <th>拍攝時間 (UTC)</th>
              <th>座標</th>
              <th>說明</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(row, idx) in rows" :key="row.file_name">
              <td class="cell-idx">{{ idx + 1 }}</td>
              <td class="cell-filename">{{ row.file_name }}</td>
              <td><span :class="sourceBadgeClass(row.source)">{{ sourceLabel(row.source) }}</span></td>
              <td class="cell-time">{{ row.capture_time_utc ?? '—' }}</td>
              <td class="cell-coord">{{ row.coord ?? '—' }}</td>
              <td class="cell-note">{{ row.note }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="modal-footer">
        <span class="sublabel">{{ rows.length }} 筆樣本，平均分布於整個資料夾</span>
        <div style="flex:1"></div>
        <button class="btn btn--outline" @click="$emit('close')">關閉</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.gpx-modal {
  width: 960px;
  height: 720px;
  display: flex;
  flex-direction: column;
}

.modal-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 18px;
  border-bottom: 1px solid #e0e0e0;
  flex-shrink: 0;
}
.modal-header h3 { margin: 0; font-size: 15px; }
.header-stats { display: flex; gap: 6px; margin-left: 8px; }
.modal-close {
  margin-left: auto;
  background: none; border: none; font-size: 20px;
  cursor: pointer; color: #666; line-height: 1;
}

/* 地圖區 */
.map-wrap {
  position: relative;
  flex: 0 0 340px;
  background: #f0f0f0;
}
.map-el { width: 100%; height: 100%; }
.map-empty {
  display: flex; align-items: center; justify-content: center;
  height: 100%; color: #888; font-size: 13px;
}
.map-legend {
  position: absolute; bottom: 8px; right: 8px;
  background: rgba(255,255,255,0.92);
  border: 1px solid #ddd;
  border-radius: 4px;
  padding: 4px 10px;
  font-size: 11px;
  display: flex; gap: 10px;
  z-index: 1000;
}
.dot {
  display: inline-block;
  width: 10px; height: 10px;
  border-radius: 50%;
  margin-right: 3px;
  vertical-align: middle;
}
.dot--green  { background: #22863a; }
.dot--yellow { background: #b08800; }
.dot--red    { background: #b3262a; }

/* 表格區 */
.preview-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
}
.preview-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
.preview-table th {
  position: sticky; top: 0;
  background: #f4f4f4;
  padding: 7px 10px;
  text-align: left;
  font-weight: 600;
  border-bottom: 2px solid #ddd;
  white-space: nowrap;
}
.preview-table td {
  padding: 5px 10px;
  border-bottom: 1px solid #eee;
  vertical-align: middle;
}
.preview-table tr:hover td { background: #f8f8f8; }

.cell-idx   { color: #999; width: 30px; text-align: center; }
.cell-filename { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cell-time  { white-space: nowrap; color: #555; }
.cell-coord { font-family: monospace; white-space: nowrap; font-size: 11px; }
.cell-note  { color: #666; font-size: 11px; }

.modal-footer {
  display: flex; align-items: center;
  padding: 10px 18px;
  border-top: 1px solid #e0e0e0;
  flex-shrink: 0;
  gap: 8px;
}

/* badges */
.badge {
  display: inline-block;
  padding: 2px 8px;
  border-radius: 10px;
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}
.badge--green  { background: #d4edda; color: #155724; }
.badge--yellow { background: #fff3cd; color: #856404; }
.badge--red    { background: #f8d7da; color: #721c24; }
</style>
