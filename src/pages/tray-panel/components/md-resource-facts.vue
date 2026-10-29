<script setup lang="ts">
import MdTooltip from '@/components/custom/md-tooltip.vue';
import MdCopyButton from '@/components/custom/md-copy-button.vue';
withDefaults(
  defineProps<{
    active?: boolean;
    layout?: 'tiles' | 'rows';
    facts: {
      key: string;
      label: string;
      value: string;
      unit: string;
      copy?: boolean;
      truncate?: boolean;
      hint?: string;
      liveHint?: boolean;
      tone?: 'success' | 'warning' | 'critical';
    }[];
  }>(),
  { active: true, layout: 'tiles' }
);
</script>
<template>
  <dl v-if="facts.length" class="resource-facts" :class="{ compact: facts.length === 1, rows: layout === 'rows' }">
    <div
      v-for="fact in facts"
      :key="fact.key"
      class="resource-fact"
      :class="{ 'resource-fact--truncate': fact.truncate }"
    >
      <dt>
        <MdTooltip v-if="fact.hint" :active="active" :text="fact.hint" :close-on-text-change="!fact.liveHint" multiline
          ><span tabindex="0">{{ fact.label }}</span></MdTooltip
        >
        <span v-else>{{ fact.label }}</span>
      </dt>
      <dd :data-tone="fact.tone">
        <MdCopyButton
          v-if="fact.copy && fact.value !== '—'"
          :active="active"
          :text="fact.value"
          :label="fact.label"
          :truncate="fact.truncate"
          show-text
        />
        <template v-else
          >{{ fact.value }}<small v-if="fact.unit">{{ fact.unit }}</small></template
        >
      </dd>
    </div>
  </dl>
</template>
<style scoped>
@reference "@assets/main.css";
.resource-facts {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(90px, 100%), 1fr));
  gap: 8px;
  flex: none;
}
.resource-fact {
  @apply rounded-lg bg-muted/40;
  padding: 8px;
  min-width: 0;
}
dt {
  @apply text-muted-foreground;
  display: flex;
  min-width: 0;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  font-size: 10px;
}
dt > span:first-child {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
dd {
  margin-top: 6px;
  font-size: 18px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
dd[data-tone='success'] {
  @apply text-success-foreground;
}
dd[data-tone='warning'] {
  @apply text-warning-foreground;
}
dd[data-tone='critical'] {
  @apply text-destructive-text;
}
.resource-fact dd[data-tone] small {
  color: inherit;
}
small {
  margin-left: 3px;
  font-size: 10px;
  font-weight: normal;
}
.compact .resource-fact {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.compact dd {
  margin-top: 0;
  flex: none;
}
.rows {
  display: flex;
  flex-direction: column;
  gap: 0;
}
.rows .resource-fact {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding: 4px 0;
  border-radius: 0;
  background: transparent;
}
.rows dt {
  font-size: 11px;
  flex: none;
}
.rows dd {
  margin: 0;
  font-size: 12px;
  font-weight: 500;
  text-align: right;
  max-width: 100%;
  min-width: 0;
  margin-left: auto;
  overflow-wrap: anywhere;
}
.rows .resource-fact--truncate {
  flex-wrap: nowrap;
}
.rows .resource-fact--truncate dd {
  flex: 1;
}
</style>
