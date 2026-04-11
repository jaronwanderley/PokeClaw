<script setup lang="ts">
// T02 will populate with chat composable and demo data
</script>

<template>
  <div class="app-shell">
    <div class="tb">
      <span class="tb-title">PokeClaw</span>
    </div>
    <div class="chat">
      <!-- Chat messages rendered here by T02 -->
    </div>
    <div class="ia">
      <!-- Message input rendered here by T02 -->
    </div>
  </div>
</template>

<style>
/* Reset */
*,
*::before,
*::after {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  height: 100%;
  width: 100%;
  overflow: hidden;
  background: #1a1a1a;
  color: #e0e0e0;
  font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
}

#app {
  height: 100%;
  width: 100%;
}

.app-shell {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
}

/* Title bar — fixed top */
.tb {
  flex-shrink: 0;
  height: 40px;
  display: flex;
  align-items: center;
  padding: 0 12px;
  background: #242424;
  border-bottom: 1px solid #333;
  user-select: none;
}

.tb-title {
  font-weight: 600;
  font-size: 13px;
  color: #ff6b35;
}

/* Chat area — scrollable middle, fills remaining space */
.chat {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
}

/* Input area — fixed bottom */
.ia {
  flex-shrink: 0;
  padding: 8px 12px;
  background: #242424;
  border-top: 1px solid #333;
}
</style>
