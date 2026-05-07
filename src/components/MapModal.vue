<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import L from 'leaflet'
import 'leaflet/dist/leaflet.css'

// Fix Leaflet default icon path issue in Vite
import iconUrl from 'leaflet/dist/images/marker-icon.png'
import iconRetinaUrl from 'leaflet/dist/images/marker-icon-2x.png'
import shadowUrl from 'leaflet/dist/images/marker-shadow.png'

delete (L.Icon.Default.prototype as any)._getIconUrl
L.Icon.Default.mergeOptions({ iconUrl, iconRetinaUrl, shadowUrl })

const props = defineProps<{ initLat: number; initLon: number }>()
const emit = defineEmits<{
  confirm: [lat: number, lon: number]
  close: []
}>()

const mapEl = ref<HTMLElement | null>(null)
const lat = ref(props.initLat)
const lon = ref(props.initLon)

let map: L.Map | null = null
let marker: L.Marker | null = null

onMounted(() => {
  if (!mapEl.value) return

  map = L.map(mapEl.value).setView([lat.value, lon.value], 13)
  L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
    attribution: '© OpenStreetMap contributors',
    maxZoom: 19,
  }).addTo(map)

  marker = L.marker([lat.value, lon.value], { draggable: true }).addTo(map)
  marker.bindPopup(`${lat.value.toFixed(6)}, ${lon.value.toFixed(6)}`).openPopup()

  marker.on('dragend', () => {
    const pos = marker!.getLatLng()
    lat.value = parseFloat(pos.lat.toFixed(6))
    lon.value = parseFloat(pos.lng.toFixed(6))
    marker!.setPopupContent(`${lat.value}, ${lon.value}`).openPopup()
  })

  map.on('click', (e: L.LeafletMouseEvent) => {
    lat.value = parseFloat(e.latlng.lat.toFixed(6))
    lon.value = parseFloat(e.latlng.lng.toFixed(6))
    marker!.setLatLng([lat.value, lon.value])
      .setPopupContent(`${lat.value}, ${lon.value}`).openPopup()
  })
})

onUnmounted(() => { map?.remove() })

function updateFromInput() {
  marker?.setLatLng([lat.value, lon.value])
  map?.setView([lat.value, lon.value])
}

function confirm() { emit('confirm', lat.value, lon.value) }
</script>

<template>
  <div class="modal-overlay" @click.self="$emit('close')">
    <div class="modal" style="width: 820px; height: 620px;">
      <div class="modal-header">
        <h3>選擇拍攝地點</h3>
        <button class="modal-close" @click="$emit('close')">×</button>
      </div>

      <p class="map-hint">在地圖上點擊或拖曳標記來選擇座標</p>

      <div ref="mapEl" class="map-container"></div>

      <div class="modal-footer">
        <span class="sublabel">緯度</span>
        <input class="input w120" type="number" v-model.number="lat" step="0.000001" @change="updateFromInput" />
        <span class="sublabel">經度</span>
        <input class="input w120" type="number" v-model.number="lon" step="0.000001" @change="updateFromInput" />
        <div style="flex:1"></div>
        <button class="btn btn--gray" @click="$emit('close')">取消</button>
        <button class="btn btn--blue" @click="confirm">確認座標</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.map-hint { font-size: 12px; color: #666; padding: 6px 18px; background: #f9f9f9; }
.map-container { flex: 1; min-height: 0; }
.modal { height: 580px; }
</style>
