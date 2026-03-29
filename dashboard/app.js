// -- Tab navigation --

function showTab(name) {
  document.querySelectorAll('.page').forEach(p => p.classList.add('hidden'));
  document.querySelectorAll('.nav-tab').forEach(a => a.classList.remove('active'));
  const page = document.getElementById('page-' + name);
  if (page) page.classList.remove('hidden');
  const tab = document.getElementById('tab-' + name);
  if (tab) tab.classList.add('active');
  if (name === 'profile') {
    location.hash = 'profile/' + (currentProfileId || '');
    fetchProfile();
  } else {
    location.hash = name;
  }
}

// -- Cookie helper --

function getCookie(name) {
  const m = document.cookie.match(new RegExp('(?:^|; )' + name + '=([^;]*)'));
  return m ? decodeURIComponent(m[1]) : null;
}

// -- Auth state --

const loggedInId = getCookie('walker_id');
let currentProfileId = new URLSearchParams(location.search).get('id') || loggedInId;

const navUser = document.getElementById('nav-user');
if (loggedInId) {
  navUser.innerHTML = '<a href="javascript:void(0)" onclick="showProfile(\'' + loggedInId + '\')" class="text-walker-500 hover:text-walker-600 font-medium">Profile</a>';
} else {
  navUser.innerHTML = '<a href="/auth/web/github" class="text-walker-500 hover:text-walker-600 font-medium">Login with GitHub</a>';
}

// -- Leaderboard --

function rankBadge(i) {
  if (i === 0) return '<span class="text-yellow-400 font-bold">1</span>';
  if (i === 1) return '<span class="text-gray-400 font-bold">2</span>';
  if (i === 2) return '<span class="text-amber-700 font-bold">3</span>';
  return '<span class="text-gray-600">' + (i + 1) + '</span>';
}

function statusIndicator(e) {
  if (e.status === 'walking') {
    return '<span class="inline-block w-2 h-2 rounded-full bg-green-500 mr-1.5 animate-pulse"></span>' +
      '<span class="text-green-400 text-xs">' + e.speed_mph.toFixed(1) + ' mph</span>';
  }
  if (e.status === 'idle') {
    return '<span class="inline-block w-2 h-2 rounded-full bg-yellow-500 mr-1.5"></span>' +
      '<span class="text-yellow-500 text-xs">Idle</span>';
  }
  return '';
}

function renderLeaderboard(elementId, entries) {
  const el = document.getElementById(elementId);
  if (!entries || entries.length === 0) {
    el.innerHTML = '<div class="text-gray-600 text-sm italic">No data yet</div>';
    return;
  }
  el.innerHTML = entries.map((e, i) => `
    <div class="flex items-center gap-3 py-2 ${i > 0 ? 'border-t border-gray-800/50' : ''}">
      <div class="w-6 text-right text-sm">${rankBadge(i)}</div>
      ${e.avatar_url
        ? '<img class="w-8 h-8 rounded-full ring-2 ring-gray-700" src="' + e.avatar_url + '" alt="">'
        : '<div class="w-8 h-8 rounded-full bg-gray-700 flex items-center justify-center text-xs font-bold text-gray-400">' + e.name[0].toUpperCase() + '</div>'
      }
      <div class="flex-1 min-w-0">
        <div class="font-medium text-sm text-gray-200 truncate cursor-pointer hover:text-white" onclick="showProfile('${e.id}')">${e.name}</div>
        <div class="flex items-center gap-1 mt-0.5">${statusIndicator(e)}</div>
      </div>
      <div class="text-right">
        <div class="text-lg font-bold text-white">${e.calories_kcal.toFixed(1)}</div>
        <div class="text-[10px] text-gray-500 -mt-0.5">kcal</div>
      </div>
    </div>
  `).join('');
}

function fetchLeaderboard() {
  fetch('/api/leaderboard')
    .then(r => r.json())
    .then(data => {
      renderLeaderboard('lb-today', data.today);
      renderLeaderboard('lb-weekly', data.weekly);
      renderLeaderboard('lb-alltime', data.all_time);
      if (window.twemoji) twemoji.parse(document.getElementById('page-leaderboard'));
    })
    .catch(() => {});
}

// -- Profile --

function showProfile(id) {
  currentProfileId = id;
  showTab('profile');
}

function formatDuration(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return h + 'h ' + m + 'm';
  return m + 'm';
}

function fetchProfile() {
  if (!currentProfileId) {
    document.getElementById('profile-content').innerHTML =
      '<div class="text-gray-600 italic text-center py-12"><a href="/auth/web/github" class="text-walker-500 hover:text-walker-600 font-medium">Login with GitHub</a> to see your stats.</div>';
    return;
  }
  fetch('/api/profile/' + encodeURIComponent(currentProfileId))
    .then(r => r.json())
    .then(renderProfile)
    .catch(() => {});
}

function buildHeatmap(days) {
  // Build a map of date → calories for quick lookup.
  const dataMap = {};
  let maxCal = 0;
  days.forEach(d => {
    dataMap[d.date] = d;
    if (d.calories_kcal > maxCal) maxCal = d.calories_kcal;
  });

  const colors = ['bg-gray-600', 'bg-green-900', 'bg-green-700', 'bg-green-500', 'bg-green-400'];
  const goldColor = 'bg-amber-400';
  const goldThresholdKm = 8.0; // ~10,000 steps — research-backed daily goal.

  // Generate exactly 53 weeks of dates ending this week.
  const today = new Date();
  const cells = [];

  // Start 53 weeks ago, aligned to Monday.
  const startDate = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const dayOfWeek = startDate.getDay();
  const mondayOffset = dayOfWeek === 0 ? 6 : dayOfWeek - 1; // Monday = 0 offset
  startDate.setDate(startDate.getDate() - (52 * 7) - mondayOffset);

  const months = [];
  let lastMonth = -1;

  // Generate all days from startDate to end of this week.
  const endDate = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  // End today — no future days.

  const d = new Date(startDate);
  while (d <= endDate) {
    const dateStr = d.getFullYear() + '-' +
      String(d.getMonth() + 1).padStart(2, '0') + '-' +
      String(d.getDate()).padStart(2, '0');
    const data = dataMap[dateStr];
    const cal = data ? data.calories_kcal : 0;

    let level = 0;
    if (cal > 0 && maxCal > 0) {
      const ratio = cal / maxCal;
      if (ratio > 0.75) level = 4;
      else if (ratio > 0.5) level = 3;
      else if (ratio > 0.25) level = 2;
      else level = 1;
    }

    const month = d.getMonth();
    if (month !== lastMonth) {
      months.push({ week: Math.floor(cells.length / 7), name: d.toLocaleString('default', { month: 'short' }) });
      lastMonth = month;
    }

    const tooltip = data
      ? dateStr + ': ' + data.calories_kcal.toFixed(1) + ' kcal, ' + data.distance_km.toFixed(2) + ' km'
      : dateStr + ': no activity';

    const isGold = data && data.distance_km >= goldThresholdKm;
    const color = isGold ? goldColor : colors[level];
    cells.push({ dateStr, level, color, tooltip, day: d.getDay() });

    d.setDate(d.getDate() + 1);
  }

  // Build grid: 7 rows (days) × ~53 columns (weeks).
  const weeks = [];
  for (let i = 0; i < cells.length; i += 7) {
    weeks.push(cells.slice(i, i + 7));
  }
  let html = '<div class="overflow-visible">';

  // Month labels — position each label at the correct week column.
  const totalWeeks = weeks.length;
  const sq = 18; // square size in px
  const gap = 3;
  const cellSize = sq + gap;
  html += '<div class="relative mb-1 text-[11px] text-gray-500" style="height: 16px; margin-left: ' + (28 + gap) + 'px; width: ' + (totalWeeks * cellSize) + 'px">';
  let lastLabelX = -50;
  months.forEach(m => {
    const x = m.week * cellSize;
    if (m.week < totalWeeks && x - lastLabelX >= 30) {
      html += '<div class="absolute" style="left: ' + x + 'px">' + m.name + '</div>';
      lastLabelX = x;
    }
  });
  html += '</div>';

  // Grid — CSS grid for uniform spacing.
  const dayLabels = ['Mon', '', 'Wed', '', 'Fri', '', ''];
  const cols = weeks.length + 1; // +1 for day labels column
  html += '<div style="display:grid; grid-template-columns: 28px repeat(' + weeks.length + ', ' + sq + 'px); grid-template-rows: repeat(7, ' + sq + 'px); gap: ' + gap + 'px">';

  // Day labels in first column.
  dayLabels.forEach((l, row) => {
    html += '<div class="text-[11px] text-gray-500 flex items-center" style="grid-column:1; grid-row:' + (row+1) + '">' + l + '</div>';
  });

  // Week columns.
  weeks.forEach((week, col) => {
    week.forEach((cell, row) => {
      if (!cell.tooltip) {
        html += '<div class="rounded-sm ' + cell.color + '" style="grid-column:' + (col+2) + '; grid-row:' + (row+1) + '"></div>';
        return;
      }
      const data = dataMap[cell.dateStr];
      const food = data ? buildFoodEquiv(data.calories_kcal, false) : '';
      const tooltipLines = cell.tooltip.replace(': ', '<br>').replace(', ', '<br>');
      const showBelow = row < 3;
      const posY = showBelow ? 'top-full mt-2' : 'bottom-full mb-2';
      const nearRight = col > totalWeeks - 12;
      const nearLeft = col < 6;
      const posX = nearRight ? 'right-0' : nearLeft ? 'left-0' : 'left-1/2 -translate-x-1/2';

      html += '<div class="relative group rounded-sm ' + cell.color + '" style="grid-column:' + (col+2) + '; grid-row:' + (row+1) + '">';
      html += '<div class="absolute ' + posY + ' ' + posX + ' hidden group-hover:block bg-gray-900 border border-gray-700 text-white text-xs px-3 py-2 rounded-lg shadow-xl z-20" style="max-width: 200px; white-space: normal;">';
      html += tooltipLines;
      if (food) html += '<div class="mt-1 text-base flex flex-wrap">' + food + '</div>';
      html += '</div>';
      html += '</div>';
    });
  });
  html += '</div>';

  // Legend.
  html += '<div class="flex items-center gap-1.5 mt-3 text-[11px] text-gray-500" style="margin-left: ' + (28 + gap) + 'px">';
  html += '<span>Less</span>';
  colors.forEach(c => {
    html += '<div class="rounded-sm ' + c + '" style="width:' + sq + 'px;height:' + sq + 'px"></div>';
  });
  html += '<span>More</span>';
  html += '<span class="ml-3">&#127942;</span>';
  html += '<div class="w-[10px] h-[10px] rounded-[2px] ' + goldColor + '"></div>';
  html += '<span>8+ km</span>';
  html += '</div>';

  html += '</div>';
  return html;
}

// Sorted largest to smallest — greedy "coin change" algorithm.
// Capped at Coca-Cola size so you see more emojis = more accomplishment.
const foodItems = [
  { emoji: '🥤', name: 'Coca-Cola 33cl', kcal: 139 },
  { emoji: '🍪', name: 'Oreo cookie', kcal: 53 },
  { emoji: '🍬', name: 'Marshmallow', kcal: 23 },
  { emoji: '🍭', name: 'Lollipop', kcal: 11 },
];

function buildFoodEquiv(kcal, compact) {
  if (kcal <= 0) return '';
  let remaining = kcal;
  let html = '';
  const maxPerItem = compact ? 5 : 30;

  for (const f of foodItems) {
    const count = Math.floor(remaining / f.kcal);
    if (count > 0) {
      remaining -= count * f.kcal;
      if (count <= maxPerItem) {
        for (let i = 0; i < count; i++) {
          html += '<span class="cursor-default inline-block hover:scale-125 transition-transform" title="' + f.name + ' (' + f.kcal + ' kcal)">' + f.emoji + '</span>';
        }
      } else {
        html += '<span class="cursor-default inline-block" title="' + count + '× ' + f.name + ' (' + f.kcal + ' kcal each)">';
        html += '<span class="text-xl">' + f.emoji + '</span>';
        html += '<span class="text-xs text-gray-400 ml-0.5">×' + count + '</span>';
        html += '</span> ';
      }
    }
  }
  return html;
}

function buildFoodRow(label, kcal) {
  if (kcal <= 0) return '';
  const equiv = buildFoodEquiv(kcal, kcal > 2000);
  return '<div class="py-3 border-t border-gray-800/50">' +
    '<div class="flex items-center justify-between mb-1">' +
      '<div class="text-xs text-gray-500">' + label + '</div>' +
      '<div class="text-xs text-gray-600">' + kcal.toFixed(0) + ' kcal</div>' +
    '</div>' +
    '<div class="text-xl leading-relaxed flex flex-wrap">' + equiv + '</div>' +
  '</div>';
}

function renderProfile(p) {
  const el = document.getElementById('profile-content');

  const last7 = p.last_7_days || [];
  const periods = p.periods || {};

  // Live status badge.
  let liveBadge = '';
  if (p.live && p.live.status === 'walking') {
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-green-500 animate-pulse"></span><span class="text-green-400 text-sm font-medium">Walking at ' + p.live.speed_mph.toFixed(1) + ' mph</span></div>';
  } else if (p.live && p.live.status === 'idle') {
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-yellow-500"></span><span class="text-yellow-500 text-sm">Idle</span></div>';
  }

  // Weekly bars.
  const maxWeekCal = Math.max(...last7.map(d => d.calories_kcal), 0.1);
  const weekBars = last7.map(d => {
    const pct = Math.max((d.calories_kcal / maxWeekCal) * 100, 3);
    const dayName = new Date(d.date + 'T00:00:00').toLocaleDateString('en', { weekday: 'short' });
    return '<div class="flex items-center gap-2">' +
      '<div class="w-8 text-right text-[11px] text-gray-500">' + dayName + '</div>' +
      '<div class="flex-1 h-5 bg-gray-800 rounded-full overflow-hidden">' +
        '<div class="h-full bg-walker-500 rounded-full transition-all" style="width:' + pct + '%"></div>' +
      '</div>' +
      '<div class="w-16 text-right text-xs text-gray-400">' + d.calories_kcal.toFixed(1) + ' kcal</div>' +
    '</div>';
  }).join('');

  el.innerHTML = `
    <!-- Hero -->
    <div class="flex items-start gap-5 mb-8">
      ${p.avatar_url
        ? '<img class="w-20 h-20 rounded-full ring-4 ring-walker-500/20" src="' + p.avatar_url + '" alt="">'
        : '<div class="w-20 h-20 rounded-full bg-gray-700 flex items-center justify-center text-3xl font-bold text-gray-400 ring-4 ring-walker-500/20">' + p.name[0].toUpperCase() + '</div>'
      }
      <div>
        <div class="text-3xl font-extrabold text-white">${p.name}</div>
        ${p.streak > 0 ? '<div class="flex items-center gap-1.5 mt-1"><span class="text-amber-400 text-lg">&#128293;</span><span class="text-amber-400 font-bold text-lg">' + p.streak + '</span><span class="text-amber-400/70 text-sm">day streak</span></div>' : ''}
        ${liveBadge}
      </div>
    </div>

    <!-- Stats grid -->
    <div class="grid grid-cols-2 md:grid-cols-4 gap-3 mb-8">
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${p.totals.calories_kcal.toFixed(1)}</div>
        <div class="text-xs text-gray-500 mt-1">Total kcal</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${p.totals.distance_km.toFixed(2)}</div>
        <div class="text-xs text-gray-500 mt-1">Total km</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${formatDuration(p.totals.active_secs)}</div>
        <div class="text-xs text-gray-500 mt-1">Total active time</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${p.totals.active_days}</div>
        <div class="text-xs text-gray-500 mt-1">Active days</div>
      </div>
    </div>

    <!-- Personal records -->
    <div class="grid grid-cols-3 gap-3 mb-8">
      <div class="bg-surface-800 rounded-xl p-4 border border-amber-900/30">
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (kcal)</div>
        <div class="text-2xl font-bold text-white">${p.records.best_day_calories_kcal.toFixed(1)}</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-amber-900/30">
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (km)</div>
        <div class="text-2xl font-bold text-white">${p.records.best_day_distance_km.toFixed(2)}</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-amber-900/30">
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (time)</div>
        <div class="text-2xl font-bold text-white">${formatDuration(p.records.best_day_active_secs)}</div>
      </div>
    </div>

    <!-- You Burned -->
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-8">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-3">You Burned</h3>
      ${buildFoodRow('Today', periods.today_kcal || 0)}
      ${buildFoodRow('This Week', periods.week_kcal || 0)}
      ${buildFoodRow('This Month', periods.month_kcal || 0)}
      ${buildFoodRow('This Year', periods.year_kcal || 0)}
      ${buildFoodRow('All Time', periods.all_time_kcal || 0)}
    </div>

    <!-- Heatmap -->
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-8 overflow-visible">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Walking Activity</h3>
      ${buildHeatmap(p.heatmap)}
    </div>

    <!-- Last 7 days -->
    ${last7.length > 0 ? `
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Last 7 Days</h3>
      <div class="space-y-2">${weekBars}</div>
    </div>
    ` : ''}
  `;

  // Render emojis consistently with Twemoji.
  if (window.twemoji) twemoji.parse(el);
}

// -- WebSocket + polling --

function connect() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(proto + '//' + location.host + '/ws/live');
  const status = document.getElementById('connection-status');

  ws.onopen = () => {
    status.textContent = 'Connected';
    status.className = 'text-xs px-6 py-1 text-green-500';
  };

  let pendingRefresh = false;
  ws.onmessage = () => {
    if (!pendingRefresh) {
      pendingRefresh = true;
      setTimeout(() => { fetchLeaderboard(); pendingRefresh = false; }, 5000);
    }
  };

  ws.onclose = () => {
    status.textContent = 'Reconnecting...';
    status.className = 'text-xs px-6 py-1 text-red-400';
    setTimeout(connect, 2000);
  };

  ws.onerror = () => ws.close();
}

// -- Restore tab from URL hash (runs immediately, before async) --
const hash = location.hash.slice(1);
if (hash.startsWith('profile/')) {
  currentProfileId = hash.split('/')[1] || currentProfileId;
  showTab('profile');
} else if (hash === 'profile' && currentProfileId) {
  showTab('profile');
} else {
  showTab('leaderboard');
}

connect();
fetchLeaderboard();
