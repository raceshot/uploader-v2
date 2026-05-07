<script setup lang="ts">
interface GpxPreviewRow {
  file_name: string
  capture_time_utc: string | null
  source: string
  coord: string | null
  note: string
}

defineProps<{ rows: GpxPreviewRow[] }>()
defineEmits<{ close: [] }>()

function sourceLabel(source: string): string {
  switch (source) {
    case 'gpx': return 'GPX'
    case 'manual-fallback': return '手動'
    case 'unmatched': return '未匹配'
    default: return source
  }
}

function sourceClass(source: string): string {
  switch (source) {
    case 'gpx': return 'badge badge--green'
    case 'manual-fallback': return 'badge badge--yellow'
    case 'unmatched': return 'badge badge--red'
    default: return 'badge'
  }
}
</script>

<template>
  <div class="modal-overlay" @click.self="$emit('close')">
    <div class="modal" style="width: 900px; height: 600px;">
      <div class="modal-header">
        <h3>GPX 抽樣預覽</h3>
        <button class="modal-close" @click="$emit('close')">×</button>
      </div>

      <div class="preview-body">
        <table class="preview-table">
          <thead>
            <tr>
              <th>檔名</th>
              <th>來源</th>
              <th>拍攝時間 (UTC)</th>
              <th>座標</th>
              <th>說明</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.file_name">
              <td class="cell-filename">{{ row.file_name }}</td>
              <td><span :class="sourceClass(row.source)">{{ sourceLabel(row.source) }}</span></td>
              <td class="cell-time">{{ row.capture_time_utc ?? '—' }}</td>
              <td class="cell-coord">{{ row.coord ?? '—' }}</td>
              <td class="cell-note">{{ row.note }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="modal-footer">
        <span class="sublabel">共 {{ rows.length }} 筆樣本</span>
        <div style="flex:1"></div>
        <button class="btn btn--blue" @click="$emit('close')">關閉</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.preview-body {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 0 18px;
}

.preview-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.preview-table th {
  position: sticky;
  top: 0;
  background: #f0f0f0;
  padding: 8px 10px;
  text-align: left;
  font-weight: 600;
  border-bottom: 2px solid #ddd;
  white-space: nowrap;
}

.preview-table td {
  padding: 6px 10px;
  border-bottom: 1px solid #eee;
  vertical-align: middle;
}

.preview-table tr:hover td { background: #f8f8f8; }

.cell-filename { max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cell-time { white-space: nowrap; color: #555; }
.cell-coord { font-family: monospace; white-space: nowrap; }
.cell-note { color: #666; }

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
