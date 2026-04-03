// -- Page initialization (determined by URL, no client-side routing) --

function initPage() {
  const path = location.pathname;

  // Migrate legacy hash URLs.
  if (location.hash.length > 1) {
    const hash = location.hash.slice(1);
    if (hash.startsWith('profile/') || hash.startsWith('activity/') || hash === 'activity') {
      location.replace('/' + hash);
      return;
    }
  }

  let page = 'leaderboard';
  if (path.startsWith('/profile/')) {
    currentProfileId = path.split('/')[2] || null;
    page = 'profile';
  } else if (path.startsWith('/activity')) {
    currentActivityId = path.split('/')[2] || null;
    page = 'activity';
  }

  // Profile and activity require login.
  if ((page === 'profile' || page === 'activity') && !loggedInId) {
    location.replace('/');
    return;
  }

  // Show the active page and tab.
  document.querySelectorAll('.page').forEach(p => p.classList.add('hidden'));
  document.querySelectorAll('.nav-tab').forEach(a => a.classList.remove('active'));
  const pageEl = document.getElementById('page-' + page);
  if (pageEl) pageEl.classList.remove('hidden');
  const tabEl = document.getElementById('tab-' + page);
  if (tabEl) tabEl.classList.add('active');

  if (page === 'profile') {
    fetchProfile();
  } else if (page === 'activity') {
    if (!currentActivityId) currentActivityId = loggedInId;
    currentActivityDate = new URLSearchParams(location.search).get('date');
    // Update heading.
    const heading = document.getElementById('activity-heading');
    if (heading) {
      if (currentActivityDate) {
        const d = new Date(currentActivityDate + 'T00:00:00');
        heading.textContent = formatDate(d) + ' Activity';
      } else {
        heading.textContent = "Today's Activity";
      }
    }
    fetchActivityClosed();
    // Only connect live WebSocket for today.
    if (!currentActivityDate) connectActivityWs();
  }
}


// -- Cookie helper --

function getCookie(name) {
  const m = document.cookie.match(new RegExp('(?:^|; )' + name + '=([^;]*)'));
  return m ? decodeURIComponent(m[1]) : null;
}

// -- Auth state --

const loggedInId = getCookie('walker_id');
let currentProfileId = loggedInId;
let currentActivityId = null;
let currentActivityDate = null; // null = today
let activityWs = null;
let lastLiveSegment = null;
let loggedInAvatar = null;

const navUser = document.getElementById('nav-user');

function logout() {
  document.cookie = 'walker_id=; Path=/; Max-Age=0';
  location.href = '/';
}

function toggleUserMenu() {
  const menu = document.getElementById('user-menu');
  if (menu) menu.classList.toggle('hidden');
}

// Close menu when clicking outside.
document.addEventListener('click', (e) => {
  const menu = document.getElementById('user-menu');
  const btn = document.getElementById('avatar-btn');
  if (menu && btn && !btn.contains(e.target) && !menu.contains(e.target)) {
    menu.classList.add('hidden');
  }
});

function buildAvatarButton(avatarUrl) {
  const avatar = avatarUrl
    ? '<img class="w-8 h-8 rounded-full ring-2 ring-gray-700 hover:ring-walker-500 transition-all cursor-pointer" src="' + avatarUrl + '" alt="">'
    : '<div class="w-8 h-8 rounded-full bg-gray-700 ring-2 ring-gray-600 hover:ring-walker-500 transition-all cursor-pointer flex items-center justify-center"><svg class="w-4 h-4 text-gray-400" fill="currentColor" viewBox="0 0 20 20"><path d="M10 9a3 3 0 100-6 3 3 0 000 6zm-7 9a7 7 0 1114 0H3z"/></svg></div>';

  navUser.innerHTML =
    '<div class="relative">' +
      '<button id="avatar-btn" onclick="toggleUserMenu()" class="flex items-center">' + avatar + '</button>' +
      '<div id="user-menu" class="hidden absolute right-0 mt-2 w-44 bg-surface-800 border border-gray-700 rounded-lg shadow-xl z-50 py-1">' +
        '<a href="/profile/' + loggedInId + '" class="block px-4 py-2 text-sm text-gray-300 hover:bg-surface-900 hover:text-white">Profile</a>' +
        '<a class="block px-4 py-2 text-sm text-gray-600 cursor-not-allowed">Settings</a>' +
        '<div class="border-t border-gray-700 my-1"></div>' +
        '<a href="javascript:void(0)" onclick="logout()" class="block px-4 py-2 text-sm text-gray-400 hover:bg-surface-900 hover:text-red-400">Logout</a>' +
      '</div>' +
    '</div>';
}

if (loggedInId) {
  // Show Activity tab.
  document.getElementById('tab-activity').style.display = '';
  // Build avatar button (no avatar URL yet — will update after first profile fetch).
  buildAvatarButton(null);
  // Fetch own profile to get avatar URL.
  fetch('/api/profile/' + encodeURIComponent(loggedInId))
    .then(r => r.json())
    .then(p => {
      if (p.avatar_url) {
        loggedInAvatar = p.avatar_url;
        buildAvatarButton(p.avatar_url);
      }
    })
    .catch(() => {});
} else {
  navUser.innerHTML = '<a href="/auth/web/github" class="text-sm text-walker-500 hover:text-walker-600 font-medium">Login with GitHub</a>';
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
      '<span class="text-green-400 text-xs">' + e.speed_kmh.toFixed(1) + ' km/h</span>';
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
        <a href="/profile/${e.id}" class="font-medium text-sm text-gray-200 truncate hover:text-white block">${e.name}</a>
        <div class="flex items-center gap-1 mt-0.5">${statusIndicator(e)}</div>
      </div>
      <div class="text-right">
        <div class="text-lg font-bold text-white">${e.active_calories_kcal.toFixed(1)}</div>
        <div class="text-[10px] text-gray-500 -mt-0.5">active kcal</div>
        <div class="text-[10px] text-gray-600 -mt-0.5">${e.calories_kcal.toFixed(1)} total</div>
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
    if (d.active_calories_kcal > maxCal) maxCal = d.active_calories_kcal;
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
    const cal = data ? data.active_calories_kcal : 0;

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
      ? dateStr + ': ' + data.active_calories_kcal.toFixed(1) + ' active kcal (' + data.calories_kcal.toFixed(1) + ' total), ' + data.distance_km.toFixed(2) + ' km'
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
      const food = data ? buildFoodEquiv(data.active_calories_kcal, false) : '';
      const tooltipLines = cell.tooltip.replace(': ', '<br>').replace(', ', '<br>');
      const showBelow = row < 3;
      const posY = showBelow ? 'top-full mt-2' : 'bottom-full mb-2';
      const nearRight = col > totalWeeks - 12;
      const nearLeft = col < 6;
      const posX = nearRight ? 'right-0' : nearLeft ? 'left-0' : 'left-1/2 -translate-x-1/2';

      const isClickable = data && data.active_calories_kcal > 0 && currentProfileId;
      const tag = isClickable ? 'a' : 'div';
      const href = isClickable ? ' href="/activity/' + currentProfileId + '?date=' + cell.dateStr + '"' : '';
      html += '<' + tag + href + ' class="relative group rounded-sm ' + cell.color + (isClickable ? ' hover:ring-1 hover:ring-walker-500' : '') + '" style="grid-column:' + (col+2) + '; grid-row:' + (row+1) + '">';
      html += '<div class="absolute ' + posY + ' ' + posX + ' hidden group-hover:block bg-gray-900 border border-gray-700 text-white text-xs px-3 py-2 rounded-lg shadow-xl z-20 whitespace-nowrap pointer-events-none">';
      html += tooltipLines;
      if (food) html += '<div class="mt-1 text-base flex flex-wrap" style="max-width: 180px;">' + food + '</div>';
      html += '</div>';
      html += '</' + tag + '>';
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
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-green-500 animate-pulse"></span><span class="text-green-400 text-sm font-medium">Walking at ' + p.live.speed_kmh.toFixed(1) + ' km/h</span></div>';
  } else if (p.live && p.live.status === 'idle') {
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-yellow-500"></span><span class="text-yellow-500 text-sm">Idle</span></div>';
  }

  // Weekly bars.
  const maxWeekCal = Math.max(...last7.map(d => d.active_calories_kcal), 0.1);
  const weekBars = last7.map(d => {
    const pct = Math.max((d.active_calories_kcal / maxWeekCal) * 100, 3);
    const dayName = new Date(d.date + 'T00:00:00').toLocaleDateString('en', { weekday: 'short' });
    return '<div class="flex items-center gap-2">' +
      '<div class="w-8 text-right text-[11px] text-gray-500">' + dayName + '</div>' +
      '<div class="flex-1 h-5 bg-gray-800 rounded-full overflow-hidden">' +
        '<div class="h-full bg-walker-500 rounded-full transition-all" style="width:' + pct + '%"></div>' +
      '</div>' +
      '<div class="w-16 text-right text-xs text-gray-400">' + d.active_calories_kcal.toFixed(1) + ' kcal</div>' +
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
        <div class="text-3xl font-extrabold text-white">${p.totals.active_calories_kcal.toFixed(1)}</div>
        <div class="text-xs text-gray-500 mt-1">Active kcal</div>
        <div class="text-xs text-gray-600">${p.totals.calories_kcal.toFixed(1)} total</div>
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
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (active kcal)</div>
        <div class="text-2xl font-bold text-white">${p.records.best_day_active_calories_kcal.toFixed(1)}</div>
        <div class="text-xs text-gray-600">${p.records.best_day_calories_kcal.toFixed(1)} total</div>
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
      ${buildFoodRow('Today', periods.today_active_kcal || 0)}
      ${buildFoodRow('This Week', periods.week_active_kcal || 0)}
      ${buildFoodRow('This Month', periods.month_active_kcal || 0)}
      ${buildFoodRow('This Year', periods.year_active_kcal || 0)}
      ${buildFoodRow('All Time', periods.all_time_active_kcal || 0)}
    </div>

    <!-- Heatmap -->
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-8 overflow-visible">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Daily Heatmap</h3>
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

// -- Activity page --

function fetchActivityClosed() {
  if (!currentActivityId) return;
  const dateParam = currentActivityDate ? '?date=' + encodeURIComponent(currentActivityDate) : '';
  fetch('/api/activity/' + encodeURIComponent(currentActivityId) + dateParam)
    .then(r => r.json())
    .then(data => {
      renderClosedSegments(data.segments || []);
      // Re-render live segment — renderClosedSegments rebuilds the DOM
      // which destroys #activity-live-inner content.
      renderLiveSegment(lastLiveSegment);
    })
    .catch(() => {});
}

function connectActivityWs() {
  disconnectActivityWs();
  if (!currentActivityId) return;
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(proto + '//' + location.host + '/ws/live/' + encodeURIComponent(currentActivityId));
  ws.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      lastLiveSegment = data.segment;
      renderLiveSegment(data.segment);
    } catch (_) {}
  };
  ws.onclose = () => {
    // Reconnect if we're still on the activity page.
    if (activityWs === ws) {
      activityWs = null;
      setTimeout(() => {
        if (currentActivityId && !document.getElementById('page-activity').classList.contains('hidden')) {
          connectActivityWs();
        }
      }, 2000);
    }
  };
  ws.onerror = () => ws.close();
  activityWs = ws;
}

function disconnectActivityWs() {
  if (activityWs) {
    const ws = activityWs;
    activityWs = null;
    ws.close();
  }
  lastLiveSegment = null;
}

function renderClosedSegments(segments) {
  const el = document.getElementById('activity-closed');
  if (!el) return;

  if (segments.length === 0) {
    el.innerHTML = '';
    return;
  }

  // Group segments into sessions (gap > 60 min = separate session).
  const sessions = [];
  let currentSession = [];

  for (let i = 0; i < segments.length; i++) {
    const seg = segments[i];
    if (currentSession.length > 0) {
      const prev = currentSession[currentSession.length - 1];
      const prevEnd = new Date(prev.started_at).getTime() / 1000 + prev.duration_s;
      const thisStart = new Date(seg.started_at).getTime() / 1000;
      if (thisStart - prevEnd > 3600) {
        sessions.push(currentSession);
        currentSession = [];
      }
    }
    currentSession.push(seg);
  }
  if (currentSession.length > 0) sessions.push(currentSession);

  // Newest first: reverse sessions and segments within each session.
  sessions.reverse();
  sessions.forEach(s => s.reverse());

  let html = '';

  sessions.forEach((session, si) => {
    if (si > 0) html += '<div class="my-6"></div>';

    // Segments are reversed (newest first), so last element is earliest.
    const sessionStart = new Date(session[session.length - 1].started_at);
    const firstSeg = session[0];
    const sessionEnd = new Date(new Date(firstSeg.started_at).getTime() + firstSeg.duration_s * 1000);
    const totalCal = session.filter(s => s.moving).reduce((sum, s) => sum + s.active_calories_kcal, 0);
    const totalDist = session.filter(s => s.moving).reduce((sum, s) => sum + s.distance_m, 0);
    const totalDur = session.filter(s => s.moving).reduce((sum, s) => sum + s.duration_s, 0);

    html += '<div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-4">';
    html += '<div class="flex items-center justify-between mb-4">';
    html += '<div class="text-sm text-gray-400">' + formatDate(sessionStart) + ' · ' + formatTime(sessionStart) + ' – <span id="session-end-' + si + '">' + formatTime(sessionEnd) + '</span></div>';
    html += '<div id="session-stats-' + si + '" class="text-sm text-gray-500">' + totalCal.toFixed(1) + ' kcal · ' + (totalDist / 1000).toFixed(2) + ' km · ' + formatDurationLong(totalDur) + '</div>';
    html += '</div>';

    // Live segment placeholder at top of newest session (below header).
    if (si === 0) {
      // Store closed-segment totals for merging with live segment.
      window._sessionClosedCal = totalCal;
      window._sessionClosedDist = totalDist;
      window._sessionClosedDur = totalDur;
      html += '<div id="activity-live-inner"></div>';
    }

    session.forEach((seg, segi) => {
      if (segi > 0) {
        // Segments are reversed: prev is newer, seg is older.
        const prev = session[segi - 1];
        const segEnd = new Date(seg.started_at).getTime() / 1000 + seg.duration_s;
        const prevStart = new Date(prev.started_at).getTime() / 1000;
        const gap = prevStart - segEnd;
        if (gap > 5) {
          html += '<div class="text-center text-xs text-gray-600 py-1.5">';
          html += 'paused ' + formatDurationLong(gap);
          html += '</div>';
        }
      }
      html += renderSegmentCard(seg);
    });

    html += '</div>';
  });

  // Store most recent closed segment end time for live segment placement.
  if (segments.length > 0) {
    const last = segments[segments.length - 1];
    window._lastClosedEnd = new Date(last.started_at).getTime() / 1000 + last.duration_s;
  } else {
    window._lastClosedEnd = null;
  }

  el.innerHTML = html;
}

function renderLiveSegment(seg) {
  const outerEl = document.getElementById('activity-live');
  const innerEl = document.getElementById('activity-live-inner');

  // Clear both containers first.
  if (outerEl) outerEl.innerHTML = '';
  if (innerEl) innerEl.innerHTML = '';

  if (!seg) return;

  // Check if live segment is adjacent to the last closed segment (< 60 min gap).
  const segStart = new Date(seg.started_at).getTime() / 1000;
  const adjacent = window._lastClosedEnd && (segStart - window._lastClosedEnd) < 3600;

  let html = renderSegmentCard(seg);
  // Show pause gap below live segment if adjacent to closed segments.
  if (adjacent && window._lastClosedEnd) {
    const gap = segStart - window._lastClosedEnd;
    if (gap > 5) {
      html += '<div class="text-center text-xs text-gray-600 py-1.5">';
      html += 'paused ' + formatDurationLong(gap);
      html += '</div>';
    }
  }

  if (adjacent && innerEl) {
    innerEl.innerHTML = html;
  } else if (outerEl) {
    outerEl.innerHTML = html;
  }

  // Update the first session's header to include the live segment.
  const statsEl = document.getElementById('session-stats-0');
  const endEl = document.getElementById('session-end-0');
  if (statsEl && seg.moving) {
    const cal = (window._sessionClosedCal || 0) + seg.active_calories_kcal;
    const dist = (window._sessionClosedDist || 0) + seg.distance_m;
    const dur = (window._sessionClosedDur || 0) + seg.duration_s;
    statsEl.textContent = cal.toFixed(1) + ' kcal \u00b7 ' + (dist / 1000).toFixed(2) + ' km \u00b7 ' + formatDurationLong(dur);
  }
  if (endEl) {
    const segEnd = new Date(new Date(seg.started_at).getTime() + seg.duration_s * 1000);
    endEl.textContent = formatTime(segEnd);
  }
}

function renderSegmentCard(seg) {
  const dur = seg.duration_s;
  if (seg.moving) {
    const met = seg.met;
    const segStart = new Date(seg.started_at);
    const segEnd = new Date(segStart.getTime() + dur * 1000);
    let html = '<div class="bg-surface-900/50 rounded-lg px-4 py-2.5 border border-gray-800/50">';
    html += '<div class="segment-row text-sm">';
    if (seg.open) {
      html += '<div class="w-2.5 h-2.5 rounded-full bg-green-500 flex-shrink-0 animate-pulse" style="grid-column:1"></div>';
    }
    html += '<span class="text-gray-400" style="grid-column:2">' + formatTime(segStart) + '–' + formatTime(segEnd) + '</span>';
    html += '<span class="text-white font-medium" style="grid-column:3">' + formatDurationLong(dur) + '</span>';
    html += '<span class="text-gray-300" style="grid-column:4">' + (seg.distance_m / 1000).toFixed(2) + ' km</span>';
    html += '<span class="text-gray-300" style="grid-column:5">' + seg.active_calories_kcal.toFixed(1) + ' <span class="text-gray-600">/ ' + seg.calories_kcal.toFixed(1) + '</span> kcal</span>';
    html += '<span class="text-gray-500" style="grid-column:6">' + seg.speed_kmh.toFixed(1) + ' km/h</span>';
    html += '<span class="text-gray-600 text-xs" style="grid-column:7">MET ' + met.toFixed(1) + '</span>';
    html += '<span class="text-gray-600 text-xs" style="grid-column:8">' + seg.weight_kg.toFixed(0) + ' kg</span>';
    html += '</div>';
    html += '</div>';
    return html;
  } else {
    let html = '<div class="text-center text-xs text-yellow-600/50 py-1.5">';
    html += 'idle ' + formatDurationLong(dur);
    html += '</div>';
    return html;
  }
}

function formatDurationLong(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0 && m > 0) return h + 'h ' + m + ' min';
  if (h > 0) return h + ' hour' + (h > 1 ? 's' : '');
  if (m > 0 && s > 0) return m + ' min ' + s + ' sec';
  if (m > 0) return m + ' min';
  return s + ' sec';
}

function formatDate(date) {
  return date.toLocaleDateString('en', { weekday: 'long', month: 'short', day: 'numeric' });
}

function formatTime(date) {
  return date.getHours().toString().padStart(2, '0') + ':' + date.getMinutes().toString().padStart(2, '0');
}

// -- WebSocket (state changes only) + polling --

const LEADERBOARD_POLL_INTERVAL_MS = 5000;

function connect() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(proto + '//' + location.host + '/ws/live');

  // WebSocket only fires on state changes (segment open/close/disconnect).
  ws.onmessage = () => {
    // Refetch closed segments — a segment was just opened or closed.
    if (currentActivityId) fetchActivityClosed();
    fetchLeaderboard();
  };

  ws.onclose = () => {
    setTimeout(connect, 2000);
  };

  ws.onerror = () => ws.close();
}

// Leaderboard polls on its own schedule, independent of WebSocket.
setInterval(fetchLeaderboard, LEADERBOARD_POLL_INTERVAL_MS);

// -- Init --
initPage();

connect();
fetchLeaderboard();
