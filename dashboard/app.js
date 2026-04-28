// -- XSS protection --

function esc(s) {
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}

function fmtNum(n, decimals) {
  return n.toLocaleString('en', { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
}

const SHORT_DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function dayLabel(dateStr, todayStr) {
  const d = new Date(dateStr + 'T00:00:00Z');
  const name = SHORT_DAYS[d.getUTCDay()];
  if (dateStr === todayStr) return '<span class="text-walker-500 font-bold">' + name + '</span>';
  return name;
}

// -- Page initialization (determined by URL, no client-side routing) --

function initPage() {
  const path = location.pathname;

  // Migrate legacy hash URLs.
  if (location.hash.length > 1) {
    const hash = location.hash.slice(1);
    if (hash.startsWith('profile/')) {
      location.replace('/' + hash);
      return;
    }
  }

  let page = 'leaderboard';
  if (path.startsWith('/profile/')) {
    currentProfileId = path.split('/')[2] || null;
    page = 'profile';
  } else if (path.startsWith('/history')) {
    currentHistoryId = path.split('/')[2] || null;
    page = 'history';
  } else if (path === '/faq') {
    page = 'faq';
  }

  // Profile and history require login.
  if ((page === 'profile' || page === 'history') && !loggedInId) {
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

  // Show hero banner for logged-out visitors on the leaderboard.
  if (page === 'leaderboard' && !loggedInId) {
    var hero = document.getElementById('hero-banner');
    if (hero) hero.classList.remove('hidden');
  }

  if (page === 'profile') {
    fetchProfile();
  } else if (page === 'history') {
    if (!currentHistoryId) currentHistoryId = loggedInId;
    currentHistoryDate = new URLSearchParams(location.search).get('date');
    // Treat today's date the same as no date (enables live WebSocket).
    const today = new Date();
    const todayStr = today.getUTCFullYear() + '-' + String(today.getUTCMonth() + 1).padStart(2, '0') + '-' + String(today.getUTCDate()).padStart(2, '0');
    if (currentHistoryDate === todayStr) currentHistoryDate = null;
    // Update heading.
    const heading = document.getElementById('history-heading');
    if (heading) {
      if (currentHistoryDate) {
        const d = new Date(currentHistoryDate + 'T00:00:00');
        heading.textContent = formatDate(d);
      } else {
        heading.textContent = "Today";
      }
    }
    fetchHistoryClosed();
    // Only connect live WebSocket for today.
    if (!currentHistoryDate) connectHistoryWs();
  }

  return page;
}


// -- Cookie helper --

function getCookie(name) {
  const m = document.cookie.match(new RegExp('(?:^|; )' + name + '=([^;]*)'));
  return m ? decodeURIComponent(m[1]) : null;
}

function setTheme(name) {
  document.documentElement.className = document.documentElement.className.replace(/\btheme-\S+/g, '').trim() + ' theme-' + name;
  document.cookie = 'walker_theme=' + name + '; Path=/; Max-Age=31536000';
}

// -- Auth state --

const loggedInId = getCookie('walker_id');
let currentProfileId = loggedInId;
let currentHistoryId = null;
let currentHistoryDate = null; // null = today
let historyWs = null;
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
    ? '<img class="w-8 h-8 rounded-full ring-2 ring-gray-700 hover:ring-walker-500 transition-all cursor-pointer" src="' + esc(avatarUrl) + '" alt="">'
    : '<div class="w-8 h-8 rounded-full bg-gray-700 ring-2 ring-gray-600 hover:ring-walker-500 transition-all cursor-pointer flex items-center justify-center"><svg class="w-4 h-4 text-gray-400" fill="currentColor" viewBox="0 0 20 20"><path d="M10 9a3 3 0 100-6 3 3 0 000 6zm-7 9a7 7 0 1114 0H3z"/></svg></div>';

  navUser.innerHTML =
    '<div class="relative">' +
      '<button id="avatar-btn" onclick="toggleUserMenu()" class="flex items-center">' + avatar + '</button>' +
      '<div id="user-menu" class="hidden absolute right-0 mt-2 w-44 bg-surface-800 border border-gray-700 rounded-lg shadow-xl z-50 py-1">' +
        '<a href="/profile/' + loggedInId + '" class="block px-4 py-2 text-sm text-gray-300 hover:bg-surface-900 hover:text-white">Profile</a>' +
        '<div class="border-t border-gray-700 my-1"></div>' +
        '<div class="px-4 py-1.5 text-xs text-gray-500">Theme</div>' +
        '<a href="javascript:void(0)" onclick="setTheme(\'gruvbox\')" class="block px-4 py-1.5 text-sm text-gray-300 hover:bg-surface-900 hover:text-white cursor-pointer">Gruvbox</a>' +
        '<a href="javascript:void(0)" onclick="setTheme(\'c64\')" class="block px-4 py-1.5 text-sm text-gray-300 hover:bg-surface-900 hover:text-white cursor-pointer">C64</a>' +
        '<a href="javascript:void(0)" onclick="setTheme(\'material\')" class="block px-4 py-1.5 text-sm text-gray-300 hover:bg-surface-900 hover:text-white cursor-pointer">Material</a>' +
        '<div class="border-t border-gray-700 my-1"></div>' +
        '<a href="javascript:void(0)" onclick="logout()" class="block px-4 py-2 text-sm text-gray-400 hover:bg-surface-900 hover:text-red-400">Logout</a>' +
      '</div>' +
    '</div>';
}

if (loggedInId) {
  // Show History tab.
  document.getElementById('tab-history').style.display = '';
  // Build avatar button (no avatar URL yet — will update after first profile fetch).
  buildAvatarButton(null);
  // Fetch own profile to get avatar URL.
  fetch('/api/profile/' + encodeURIComponent(loggedInId))
    .then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    })
    .then(p => {
      if (p.avatar_url) {
        loggedInAvatar = p.avatar_url;
        buildAvatarButton(p.avatar_url);
      }
    })
    .catch(e => console.error('Failed to fetch profile:', e));
} else {
  navUser.innerHTML = '<a href="/login" class="text-sm text-walker-500 hover:text-walker-600 font-medium">Login</a>';
}

// -- Leaderboard --

function rankBadge(i) {
  if (i === 0) return '<span class="text-yellow-400 font-bold">1</span>';
  if (i === 1) return '<span class="text-gray-400 font-bold">2</span>';
  if (i === 2) return '<span class="text-amber-700 font-bold">3</span>';
  return '<span class="text-gray-600">' + (i + 1) + '</span>';
}

// Muted pipe separator for the live status row. Kept low-contrast so the
// themed numbers on either side remain the visual focus.
const SEP = ' <span class="text-gray-600">|</span> ';

// "no incline" for null/zero; "X.X% incline" otherwise. Applied on the
// leaderboard for a quick-glance "is this user on a climb" hint.
function inclineLabel(pct) {
  if (pct == null || pct === 0) return 'no incline';
  return pct.toFixed(1) + '% incline';
}

function statusIndicator(e) {
  if (e.status === 'walking') {
    const parts = [e.speed_kmh.toFixed(1) + ' km/h'];
    if (e.active_kcal_per_h) parts.push(e.active_kcal_per_h.toFixed(1) + ' kcal/h');
    parts.push(inclineLabel(e.incline_percent));
    return '<span class="inline-block w-2 h-2 rounded-full bg-status-walking mr-1.5 live-blink"></span>' +
      '<span class="text-status-walking text-xs">' + parts.join(SEP) + '</span>';
  }
  if (e.status === 'idle') {
    // Only surface incline when it matters (nonzero). "Idle | no incline"
    // would just be noise on a row that already says the user isn't moving.
    const suffix = (e.incline_percent != null && e.incline_percent !== 0)
      ? SEP + e.incline_percent.toFixed(1) + '% incline'
      : '';
    return '<span class="inline-block w-2 h-2 rounded-full bg-status-idle mr-1.5"></span>' +
      '<span class="text-status-idle text-xs">Idle' + suffix + '</span>';
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
    <div class="relative group flex items-center gap-3 py-2 ${i > 0 ? 'border-t border-gray-800/50' : ''}">
      <div class="w-6 text-right text-sm">${rankBadge(i)}</div>
      ${e.avatar_url
        ? '<img class="w-8 h-8 rounded-full ring-2 ring-gray-700" src="' + esc(e.avatar_url) + '" alt="">'
        : '<div class="w-8 h-8 rounded-full bg-gray-700 flex items-center justify-center text-xs font-bold text-gray-400">' + esc(e.name[0].toUpperCase()) + '</div>'
      }
      <div class="flex-1 min-w-0">
        <a href="/profile/${e.id}" class="font-medium text-sm text-gray-200 truncate hover:text-white block">${esc(e.name)}</a>
        <div class="flex items-center gap-1 mt-0.5">${statusIndicator(e)}</div>
      </div>
      <div class="text-right shrink-0 text-sm font-bold text-white">${e.active_calories_kcal.toFixed(1)} kcal</div>
      <div class="absolute right-0 top-full hidden group-hover:block bg-gray-900 border border-gray-700 text-white text-xs px-3 py-2 rounded-lg shadow-xl z-20 whitespace-nowrap pointer-events-none">
        ${fmtNum(e.distance_km, 2)} km
      </div>
    </div>
  `).join('');
}

function renderDailyWinners(entries) {
  const el = document.getElementById('lb-daily-winners');
  if (!entries || entries.length === 0) {
    el.innerHTML = '<div class="text-gray-600 text-sm italic">No data yet</div>';
    return;
  }
  const now = new Date();
  const todayStr = now.getUTCFullYear() + '-' + String(now.getUTCMonth() + 1).padStart(2, '0') + '-' + String(now.getUTCDate()).padStart(2, '0');
  el.innerHTML = entries.map((e, i) => {
    const isToday = e.date === todayStr;
    const day = dayLabel(e.date, todayStr);
    const status = isToday ? statusIndicator(e) : '';
    return `
      <div class="relative group flex items-center gap-3 py-2 ${i > 0 ? 'border-t border-gray-800/50' : ''}">
        <div class="w-10 text-xs text-gray-500 shrink-0">${day}</div>
        ${e.avatar_url
          ? '<img class="w-6 h-6 rounded-full ring-2 ring-gray-700 shrink-0" src="' + esc(e.avatar_url) + '" alt="">'
          : '<div class="w-6 h-6 rounded-full bg-gray-700 flex items-center justify-center text-xs font-bold text-gray-400 shrink-0">' + esc(e.name[0].toUpperCase()) + '</div>'
        }
        <div class="flex-1 min-w-0">
          <a href="/profile/${e.id}" class="font-medium text-sm text-gray-200 truncate hover:text-white block">${esc(e.name)}</a>
          <div class="flex items-center gap-1 mt-0.5">${status}</div>
        </div>
        <div class="text-right shrink-0 text-sm font-bold text-white">${e.active_calories_kcal.toFixed(1)} kcal</div>
        <div class="absolute right-0 top-full hidden group-hover:block bg-gray-900 border border-gray-700 text-white text-xs px-3 py-2 rounded-lg shadow-xl z-20 whitespace-nowrap pointer-events-none">
          ${fmtNum(e.distance_km, 2)} km
        </div>
      </div>`;
  }).join('');
}

function fetchLeaderboard() {
  fetch('/api/leaderboard')
    .then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    })
    .then(data => {
      renderLeaderboard('lb-today', data.today);
      renderLeaderboard('lb-weekly', data.weekly);
      renderLeaderboard('lb-alltime', data.all_time);
      renderDailyWinners(data.daily_winners);
      if (window.twemoji) twemoji.parse(document.getElementById('page-leaderboard'));
    })
    .catch(e => console.error('Failed to fetch leaderboard:', e));
}

// -- Day chart (cumulative kcal over time, per user) --

let currentDayDate = null; // null = today (auto-rolls)
let lastDayData = null;

function utcTodayStr() {
  const d = new Date();
  return d.getUTCFullYear() + '-' + String(d.getUTCMonth() + 1).padStart(2, '0') + '-' + String(d.getUTCDate()).padStart(2, '0');
}

function dayViewingToday() {
  return !currentDayDate || currentDayDate === utcTodayStr();
}

function dayShiftDate(deltaDays) {
  const cur = currentDayDate || utcTodayStr();
  const d = new Date(cur + 'T00:00:00Z');
  d.setUTCDate(d.getUTCDate() + deltaDays);
  const next = d.getUTCFullYear() + '-' + String(d.getUTCMonth() + 1).padStart(2, '0') + '-' + String(d.getUTCDate()).padStart(2, '0');
  // Block future dates.
  if (next > utcTodayStr()) return;
  currentDayDate = next === utcTodayStr() ? null : next;
  fetchDay();
}

function userColor(id) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
  return 'hsl(' + ((h % 360 + 360) % 360) + ', 65%, 60%)';
}

function fetchDay() {
  const date = currentDayDate || utcTodayStr();
  fetch('/api/day/' + date)
    .then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    })
    .then(data => {
      lastDayData = data;
      renderDay();
    })
    .catch(e => console.error('Failed to fetch day:', e));
}

// Build cumulative-kcal points {t, kcal} where t is seconds since UTC midnight.
function buildUserPoints(user, viewDateStr, isToday) {
  const dayStartMs = new Date(viewDateStr + 'T00:00:00Z').getTime();
  const points = [{t: 0, kcal: 0}];
  let cum = 0;
  for (const seg of user.segments) {
    const startMs = new Date(seg.started_at).getTime();
    const startT = (startMs - dayStartMs) / 1000;
    let durSec = seg.duration_s;
    let kcal = seg.active_calories_kcal;
    if (seg.open && isToday) {
      const elapsed = (Date.now() - startMs) / 1000;
      if (elapsed > durSec && durSec > 0.01) {
        kcal = (kcal / durSec) * elapsed;
        durSec = elapsed;
      }
    }
    const endT = startT + durSec;
    if (points[points.length - 1].t < startT) points.push({t: startT, kcal: cum});
    cum += kcal;
    points.push({t: endT, kcal: cum});
  }
  // Extend flat to the right edge: 24:00 for past days, "now" for today.
  const rightEdge = isToday ? Math.min(86400, (Date.now() - dayStartMs) / 1000) : 86400;
  if (points[points.length - 1].t < rightEdge) points.push({t: rightEdge, kcal: cum});
  return points;
}

function kcalAtTime(points, t) {
  if (t <= points[0].t) return points[0].kcal;
  for (let i = 1; i < points.length; i++) {
    if (t <= points[i].t) {
      const a = points[i-1], b = points[i];
      if (b.t === a.t) return b.kcal;
      return a.kcal + (b.kcal - a.kcal) * (t - a.t) / (b.t - a.t);
    }
  }
  return points[points.length - 1].kcal;
}

function niceMax(v) {
  if (v <= 0) return 10;
  const exp = Math.pow(10, Math.floor(Math.log10(v)));
  const f = v / exp;
  let nf;
  if (f <= 1) nf = 1;
  else if (f <= 2) nf = 2;
  else if (f <= 5) nf = 5;
  else nf = 10;
  return nf * exp;
}

function fmtTimeOfDay(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return String(h).padStart(2, '0') + ':' + String(m).padStart(2, '0');
}

function renderDay() {
  const data = lastDayData;
  if (!data) return;
  const isToday = dayViewingToday();
  const viewDate = data.date;

  // Title.
  const titleEl = document.getElementById('day-title');
  if (isToday) {
    titleEl.textContent = 'Today';
  } else {
    const d = new Date(viewDate + 'T00:00:00Z');
    titleEl.textContent = d.toLocaleDateString('en', { timeZone: 'UTC', weekday: 'long', month: 'short', day: 'numeric' });
  }

  // Buttons: disable next/today when on today.
  document.getElementById('day-next').disabled = isToday;
  document.getElementById('day-next').classList.toggle('opacity-30', isToday);
  document.getElementById('day-next').classList.toggle('cursor-not-allowed', isToday);
  document.getElementById('day-today').disabled = isToday;
  document.getElementById('day-today').classList.toggle('opacity-30', isToday);
  document.getElementById('day-today').classList.toggle('cursor-not-allowed', isToday);

  // Build per-user series.
  const series = data.users.map(u => ({
    id: u.id,
    name: u.name,
    avatar_url: u.avatar_url,
    color: userColor(u.id),
    points: buildUserPoints(u, viewDate, isToday),
  }));

  // Empty state.
  const container = document.getElementById('day-chart-container');
  const legendEl = document.getElementById('day-chart-legend');
  if (series.length === 0) {
    container.innerHTML = '<div class="h-[280px] flex items-center justify-center text-gray-600 italic text-sm">No activity</div>';
    legendEl.innerHTML = '<div class="text-xs text-gray-600 italic">No active users</div>';
    return;
  }

  // Dimensions.
  const W = container.clientWidth || 1200;
  const H = 280;
  const PAD = { l: 44, r: 14, t: 12, b: 28 };
  const innerW = W - PAD.l - PAD.r;
  const innerH = H - PAD.t - PAD.b;

  // X domain: crop to activity, snapped to hour, with a 4h minimum.
  // Derive firstAct from raw segment data so a segment starting at exactly 00:00 UTC
  // is honored (points[1] would be the segment's endpoint in that case, not its start).
  const dayStartMs = new Date(viewDate + 'T00:00:00Z').getTime();
  const firstAct = Math.min(...data.users.map(u =>
    u.segments.length > 0
      ? (new Date(u.segments[0].started_at).getTime() - dayStartMs) / 1000
      : 86400
  ));
  const lastAct = Math.max(...series.map(s => s.points[s.points.length - 1].t));
  let xMin = Math.max(0, Math.floor(firstAct / 3600) * 3600);
  let xMax = Math.min(86400, Math.ceil(lastAct / 3600) * 3600);
  const MIN_DOMAIN = 4 * 3600;
  if (xMax - xMin < MIN_DOMAIN) {
    const pad = (MIN_DOMAIN - (xMax - xMin)) / 2;
    xMin = Math.max(0, xMin - pad);
    xMax = Math.min(86400, xMax + pad);
    if (xMax - xMin < MIN_DOMAIN) {
      if (xMin === 0) xMax = Math.min(86400, xMin + MIN_DOMAIN);
      else if (xMax === 86400) xMin = Math.max(0, xMax - MIN_DOMAIN);
    }
  }
  const xDomain = xMax - xMin;

  const maxKcal = Math.max(10, ...series.map(s => s.points[s.points.length - 1].kcal));
  const yMax = niceMax(maxKcal * 1.1);

  const xPx = t => PAD.l + (Math.max(xMin, Math.min(xMax, t)) - xMin) / xDomain * innerW;
  const yPx = k => PAD.t + innerH - (k / yMax) * innerH;

  // X tick step: aim for ~6 ticks at sensible hour multiples.
  let xStep;
  if (xDomain <= 6 * 3600) xStep = 3600;
  else if (xDomain <= 12 * 3600) xStep = 2 * 3600;
  else xStep = 4 * 3600;

  // Grid + axes.
  let svg = '';
  svg += '<svg width="' + W + '" height="' + H + '" class="block">';

  // Y gridlines + labels.
  const yTicks = 5;
  for (let i = 0; i <= yTicks; i++) {
    const k = (yMax / yTicks) * i;
    const y = yPx(k);
    svg += '<line x1="' + PAD.l + '" x2="' + (W - PAD.r) + '" y1="' + y + '" y2="' + y + '" stroke="rgb(var(--gray-800))" stroke-width="1"/>';
    svg += '<text x="' + (PAD.l - 6) + '" y="' + (y + 3) + '" text-anchor="end" class="fill-gray-500" style="font-size:10px">' + Math.round(k) + '</text>';
  }

  // X gridlines + labels (snapped to xStep within [xMin, xMax]).
  for (let t = xMin; t <= xMax + 1; t += xStep) {
    const x = xPx(t);
    svg += '<line x1="' + x + '" x2="' + x + '" y1="' + PAD.t + '" y2="' + (H - PAD.b) + '" stroke="rgb(var(--gray-800))" stroke-width="1"/>';
    svg += '<text x="' + x + '" y="' + (H - PAD.b + 14) + '" text-anchor="middle" class="fill-gray-500" style="font-size:10px">' + String(Math.round(t / 3600)).padStart(2, '0') + ':00</text>';
  }

  // "Now" line on today.
  if (isToday) {
    const nowSecs = (Date.now() - new Date(viewDate + 'T00:00:00Z').getTime()) / 1000;
    if (nowSecs >= xMin && nowSecs <= xMax) {
      const x = xPx(nowSecs);
      svg += '<line x1="' + x + '" x2="' + x + '" y1="' + PAD.t + '" y2="' + (H - PAD.b) + '" stroke="rgb(var(--walker-500))" stroke-width="1" stroke-dasharray="2,3" opacity="0.5"/>';
    }
  }

  // Polylines.
  for (const s of series) {
    const pts = s.points.map(p => xPx(p.t) + ',' + yPx(p.kcal)).join(' ');
    svg += '<polyline points="' + pts + '" fill="none" stroke="' + s.color + '" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>';
  }

  // Hover crosshair (hidden by default).
  svg += '<line id="day-crosshair" x1="0" x2="0" y1="' + PAD.t + '" y2="' + (H - PAD.b) + '" stroke="rgb(var(--gray-500))" stroke-width="1" stroke-dasharray="3,3" opacity="0" pointer-events="none"/>';

  // Hover-capture rect.
  svg += '<rect id="day-hover-rect" x="' + PAD.l + '" y="' + PAD.t + '" width="' + innerW + '" height="' + innerH + '" fill="transparent"/>';

  svg += '</svg>';

  container.innerHTML = svg + '<div id="day-tooltip" class="absolute pointer-events-none hidden bg-surface-950 border border-gray-700 rounded-lg shadow-lg px-3 py-2 text-xs" style="z-index:50; min-width:180px"></div>';

  // Legend.
  legendEl.innerHTML = series.map(s =>
    '<div class="flex items-center gap-2 text-xs text-gray-300">' +
      '<span class="inline-block w-3 h-3 rounded-sm" style="background:' + s.color + '"></span>' +
      '<span>' + esc(s.name) + '</span>' +
      '<span class="text-gray-500">' + fmtNum(s.points[s.points.length - 1].kcal, 1) + ' kcal</span>' +
    '</div>'
  ).join('');

  // Hover handling.
  const rect = container.querySelector('#day-hover-rect');
  const cross = container.querySelector('#day-crosshair');
  const tip = container.querySelector('#day-tooltip');
  rect.addEventListener('mousemove', (e) => {
    const bounds = container.getBoundingClientRect();
    const px = e.clientX - bounds.left;
    const t = xMin + ((px - PAD.l) / innerW) * xDomain;
    const tClamped = Math.max(xMin, Math.min(xMax, t));
    cross.setAttribute('x1', xPx(tClamped));
    cross.setAttribute('x2', xPx(tClamped));
    cross.setAttribute('opacity', '0.7');
    const ranked = series
      .map(s => ({ name: s.name, color: s.color, kcal: kcalAtTime(s.points, tClamped) }))
      .filter(r => r.kcal > 0)
      .sort((a, b) => b.kcal - a.kcal);
    let html = '<div class="text-gray-400 mb-1">' + fmtTimeOfDay(tClamped) + ' UTC</div>';
    if (ranked.length === 0) {
      html += '<div class="text-gray-600 italic">No one walking yet</div>';
    } else {
      html += ranked.map(r =>
        '<div class="flex items-center justify-between gap-3 py-0.5">' +
          '<span class="flex items-center gap-2"><span class="inline-block w-2 h-2 rounded-sm" style="background:' + r.color + '"></span>' +
          '<span class="text-gray-200">' + esc(r.name) + '</span></span>' +
          '<span class="text-white font-medium tabular-nums">' + fmtNum(r.kcal, 1) + '</span>' +
        '</div>'
      ).join('');
    }
    tip.innerHTML = html;
    tip.classList.remove('hidden');
    // Position: near cursor, flip if too close to right edge.
    const tipW = tip.offsetWidth || 200;
    const tipH = tip.offsetHeight || 80;
    let tx = px + 14;
    if (tx + tipW > bounds.width - 4) tx = px - tipW - 14;
    let ty = e.clientY - bounds.top + 14;
    if (ty + tipH > bounds.height - 4) ty = bounds.height - tipH - 4;
    tip.style.left = tx + 'px';
    tip.style.top = ty + 'px';
  });
  rect.addEventListener('mouseleave', () => {
    cross.setAttribute('opacity', '0');
    tip.classList.add('hidden');
  });
}

// Wire navigation buttons (once).
document.getElementById('day-prev').addEventListener('click', () => dayShiftDate(-1));
document.getElementById('day-next').addEventListener('click', () => { if (!dayViewingToday()) dayShiftDate(1); });
document.getElementById('day-today').addEventListener('click', () => { if (!dayViewingToday()) { currentDayDate = null; fetchDay(); } });
window.addEventListener('resize', () => { if (lastDayData) renderDay(); });

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
      '<div class="text-gray-600 italic text-center py-12"><a href="/login" class="text-walker-500 hover:text-walker-600 font-medium">Login</a> to see your stats.</div>';
    return;
  }
  fetch('/api/profile/' + encodeURIComponent(currentProfileId))
    .then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    })
    .then(renderProfile)
    .catch(e => console.error('Failed to fetch profile:', e));
}

function buildHeatmap(days) {
  // Build a map of date → calories for quick lookup.
  const dataMap = {};
  let maxCal = 0;
  days.forEach(d => {
    dataMap[d.date] = d;
    if (d.active_calories_kcal > maxCal) maxCal = d.active_calories_kcal;
  });

  const colors = ['bg-heat-0', 'bg-heat-1', 'bg-heat-2', 'bg-heat-3', 'bg-heat-4'];
  const goldColor = 'bg-heat-gold';
  const goldThresholdKm = 8.0; // ~10,000 steps — research-backed daily goal.

  // Generate exactly 53 weeks of dates ending this week.
  // Use UTC dates to match server's CURRENT_DATE (UTC).
  const today = new Date();
  const cells = [];

  // Start 53 weeks ago, aligned to Monday.
  const startDate = new Date(Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate()));
  const dayOfWeek = startDate.getUTCDay();
  const mondayOffset = dayOfWeek === 0 ? 6 : dayOfWeek - 1; // Monday = 0 offset
  startDate.setUTCDate(startDate.getUTCDate() - (52 * 7) - mondayOffset);

  const months = [];
  let lastMonth = -1;

  // Generate all days from startDate to end of this week.
  const endDate = new Date(Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate()));
  // End today — no future days.

  const d = new Date(startDate);
  while (d <= endDate) {
    const dateStr = d.getUTCFullYear() + '-' +
      String(d.getUTCMonth() + 1).padStart(2, '0') + '-' +
      String(d.getUTCDate()).padStart(2, '0');
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

    const month = d.getUTCMonth();
    if (month !== lastMonth) {
      months.push({ week: Math.floor(cells.length / 7), name: d.toLocaleString('default', { month: 'short', timeZone: 'UTC' }) });
      lastMonth = month;
    }

    const tooltip = data
      ? dateStr + ': ' + data.active_calories_kcal.toFixed(1) + ' active kcal, ' + data.distance_km.toFixed(2) + ' km'
      : dateStr + ': no activity';

    const isGold = data && data.distance_km >= goldThresholdKm;
    const color = isGold ? goldColor : colors[level];
    cells.push({ dateStr, level, color, tooltip, day: d.getUTCDay() });

    d.setUTCDate(d.getUTCDate() + 1);
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
  html += '<div class="relative mb-1 text-[11px] text-gray-500" style="height: 16px; margin-left: calc(var(--hm-label-w, 28px) + ' + gap + 'px); width: ' + (totalWeeks * cellSize) + 'px">';
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
  html += '<div style="display:grid; grid-template-columns: var(--hm-label-w, 28px) repeat(' + weeks.length + ', ' + sq + 'px); grid-template-rows: repeat(7, ' + sq + 'px); gap: ' + gap + 'px">';

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

      const hasActivity = data && data.active_calories_kcal > 0;
      const isDev = document.cookie.split(';').some(c => c.trim() === 'walker_dev=1');
      const isOwn = currentProfileId === loggedInId;
      const isClickable = currentProfileId && isOwn && (hasActivity || isDev);
      const tag = isClickable ? 'a' : 'div';
      const href = isClickable ? ' href="/history/' + currentProfileId + '?date=' + cell.dateStr + '"' : '';
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
  html += '<div class="flex items-center gap-1.5 mt-3 text-[11px] text-gray-500" style="margin-left: calc(var(--hm-label-w, 28px) + ' + gap + 'px)">';
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
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-status-walking live-blink"></span><span class="text-status-walking text-sm font-medium">Walking at ' + p.live.speed_kmh.toFixed(1) + ' km/h</span></div>';
  } else if (p.live && p.live.status === 'idle') {
    liveBadge = '<div class="flex items-center gap-2 mt-2"><span class="inline-block w-2.5 h-2.5 rounded-full bg-status-idle"></span><span class="text-status-idle text-sm">Idle</span></div>';
  }

  // Weekly bars — fill in missing days with zeroes.
  const now = new Date();
  const todayStr = now.getUTCFullYear() + '-' + String(now.getUTCMonth() + 1).padStart(2, '0') + '-' + String(now.getUTCDate()).padStart(2, '0');
  const dataByDate = {};
  last7.forEach(d => { dataByDate[d.date] = d; });
  const allDays = [];
  for (let i = 0; i <= 6; i++) {
    const d = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - i));
    const dateStr = d.getUTCFullYear() + '-' + String(d.getUTCMonth() + 1).padStart(2, '0') + '-' + String(d.getUTCDate()).padStart(2, '0');
    allDays.push(dataByDate[dateStr] || { date: dateStr, active_calories_kcal: 0, distance_km: 0, active_secs: 0 });
  }
  const maxWeekCal = Math.max(...allDays.map(d => d.active_calories_kcal), 0.1);
  const weekBars = allDays.map(d => {
    const pct = d.active_calories_kcal >= 1 ? Math.max((d.active_calories_kcal / maxWeekCal) * 100, 1) : 0;
    const isToday = d.date === todayStr;
    const dayName = dayLabel(d.date, todayStr);
    const isLive = isToday && p.live && p.live.status === 'walking';
    const isIdle = isToday && p.live && p.live.status === 'idle';
    const liveDot = isLive
      ? '<span class="inline-block w-2.5 h-2.5 shrink-0 rounded-full bg-status-walking live-blink mr-1.5"></span>'
      : isIdle
        ? '<span class="inline-block w-2.5 h-2.5 shrink-0 rounded-full bg-status-idle mr-1.5"></span>'
        : '';
    return '<div class="flex items-center gap-2">' +
      '<div class="text-right text-sm text-gray-400 shrink-0 flex items-center justify-end" style="width: var(--bar-day-w, 32px)">' + liveDot + dayName + '</div>' +
      '<div class="flex-1 h-7 bg-gray-700 rounded-full overflow-hidden">' +
        '<div class="h-full bg-walker-500 rounded-full transition-all" style="width:' + pct + '%"></div>' +
      '</div>' +
      '<div class="text-right text-sm text-gray-400 whitespace-nowrap pl-2 shrink-0" style="width: var(--bar-kcal-w, 120px)">' + d.active_calories_kcal.toFixed(1) + ' active kcal</div>' +
    '</div>';
  }).join('');

  el.innerHTML = `
    <!-- Hero -->
    <div class="flex items-start gap-5 mb-8">
      ${p.avatar_url
        ? '<img class="w-20 h-20 rounded-full ring-4 ring-walker-500/20" src="' + esc(p.avatar_url) + '" alt="">'
        : '<div class="w-20 h-20 rounded-full bg-gray-700 flex items-center justify-center text-3xl font-bold text-gray-400 ring-4 ring-walker-500/20">' + esc(p.name[0].toUpperCase()) + '</div>'
      }
      <div>
        <div class="text-3xl font-extrabold text-white">${esc(p.name)}</div>
        ${p.email ? '<div class="text-xs text-gray-500 mt-0.5">' + esc(p.email) + '</div>' : ''}
        ${p.streak > 0 ? '<div class="flex items-center gap-1.5 mt-1"><span class="text-amber-400 text-lg">&#128293;</span><span class="text-amber-400 font-bold text-lg">' + p.streak + '</span><span class="text-amber-400/70 text-sm">day streak</span></div>' : ''}
        ${liveBadge}
      </div>
    </div>

    <!-- Last 7 days -->
    ${last7.length > 0 ? `
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-8">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Last 7 Days</h3>
      <div class="space-y-2">${weekBars}</div>
    </div>
    ` : ''}

    <!-- Heatmap -->
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-8 overflow-visible">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Daily Heatmap</h3>
      ${buildHeatmap(p.heatmap)}
    </div>

    <!-- Stats grid -->
    <div class="grid grid-cols-2 md:grid-cols-4 gap-3 mb-8">
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${fmtNum(p.totals.active_calories_kcal, 1)}</div>
        <div class="text-xs text-gray-500 mt-1">Active kcal</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-gray-800">
        <div class="text-3xl font-extrabold text-white">${fmtNum(p.totals.distance_km, 2)}</div>
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
        <div class="text-2xl font-bold text-white">${fmtNum(p.records.best_day_active_calories_kcal, 1)}</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-amber-900/30">
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (km)</div>
        <div class="text-2xl font-bold text-white">${fmtNum(p.records.best_day_distance_km, 2)}</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4 border border-amber-900/30">
        <div class="text-amber-400 text-[10px] font-semibold uppercase tracking-wider mb-1">&#127942; Best Day (time)</div>
        <div class="text-2xl font-bold text-white">${formatDuration(p.records.best_day_active_secs)}</div>
      </div>
    </div>

    <!-- You Burned -->
    <div class="bg-surface-800 rounded-xl p-5 border border-gray-800">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-3">You Burned</h3>
      ${buildFoodRow('Today', periods.today_active_kcal || 0)}
      ${buildFoodRow('This Week', periods.week_active_kcal || 0)}
      ${buildFoodRow('This Month', periods.month_active_kcal || 0)}
      ${buildFoodRow('This Year', periods.year_active_kcal || 0)}
      ${buildFoodRow('All Time', periods.all_time_active_kcal || 0)}
    </div>
  `;

  // Render emojis consistently with Twemoji.
  if (window.twemoji) twemoji.parse(el);
}

// -- History page --

function fetchHistoryClosed() {
  if (!currentHistoryId) return;
  const dateParam = currentHistoryDate ? '?date=' + encodeURIComponent(currentHistoryDate) : '';
  fetch('/api/history/' + encodeURIComponent(currentHistoryId) + dateParam)
    .then(r => {
      if (!r.ok) throw new Error(r.status);
      return r.json();
    })
    .then(data => {
      renderClosedSegments(data.segments || []);
      // Re-render live segment — renderClosedSegments rebuilds the DOM
      // which destroys #history-live-inner content.
      renderLiveSegment(lastLiveSegment);
    })
    .catch(e => console.error('Failed to fetch history:', e));
}

function connectHistoryWs() {
  disconnectHistoryWs();
  if (!currentHistoryId) return;
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(proto + '//' + location.host + '/ws/live/' + encodeURIComponent(currentHistoryId));
  ws.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      lastLiveSegment = data.segment;
      renderLiveSegment(data.segment);
    } catch (_) {}
  };
  ws.onclose = () => {
    // Reconnect if we're still on the history page.
    if (historyWs === ws) {
      historyWs = null;
      setTimeout(() => {
        if (currentHistoryId && !document.getElementById('page-history').classList.contains('hidden')) {
          connectHistoryWs();
        }
      }, 2000);
    }
  };
  ws.onerror = () => ws.close();
  historyWs = ws;
}

function disconnectHistoryWs() {
  if (historyWs) {
    const ws = historyWs;
    historyWs = null;
    ws.close();
  }
  lastLiveSegment = null;
}

function renderClosedSegments(segments) {
  const el = document.getElementById('history-closed');
  if (!el) return;

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

  const isToday = !currentHistoryDate;

  // Always create at least one session panel so there's a visible day-card.
  // For today, this also gives the live segment a home.
  if (sessions.length === 0) {
    sessions.push([]);
  }

  let html = '';

  sessions.forEach((session, si) => {
    if (si > 0) html += '<div class="my-6"></div>';

    const isEmpty = session.length === 0;

    // Segments are reversed (newest first), so last element is earliest.
    const sessionStart = isEmpty ? null : new Date(session[session.length - 1].started_at);
    const sessionEnd = isEmpty ? null : new Date(new Date(session[0].started_at).getTime() + session[0].duration_s * 1000);
    const totalCal = session.filter(s => s.moving).reduce((sum, s) => sum + s.active_calories_kcal, 0);
    const totalDist = session.filter(s => s.moving).reduce((sum, s) => sum + s.distance_m, 0);
    const totalDur = session.filter(s => s.moving).reduce((sum, s) => sum + s.duration_s, 0);

    html += '<div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-4">';
    if (!isEmpty) {
      html += '<div class="flex items-center justify-between mb-4">';
      html += '<div class="text-sm text-gray-400">' + formatDate(sessionStart) + ' · ' + formatTime(sessionStart) + ' – <span id="session-end-' + si + '">' + formatTime(sessionEnd) + '</span></div>';
      html += '<div id="session-stats-' + si + '" class="text-sm text-gray-500">' + totalCal.toFixed(1) + ' kcal · ' + (totalDist / 1000).toFixed(2) + ' km · ' + formatDurationLong(totalDur) + '</div>';
      html += '</div>';
    }

    // Live segment placeholder at top of newest session (below header).
    if (si === 0 && isToday) {
      // Store closed-segment totals for merging with live segment.
      window._sessionClosedCal = totalCal;
      window._sessionClosedDist = totalDist;
      window._sessionClosedDur = totalDur;
      html += '<div id="history-live-inner"></div>';
    }

    if (isEmpty) {
      html += '<div id="history-empty" class="text-center text-sm text-gray-600 py-4">No activity</div>';
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
  const outerEl = document.getElementById('history-live');
  const innerEl = document.getElementById('history-live-inner');

  // Clear both containers first.
  if (outerEl) outerEl.innerHTML = '';
  if (innerEl) innerEl.innerHTML = '';

  // Hide "No activity" placeholder when live segment arrives.
  const emptyEl = document.getElementById('history-empty');
  if (emptyEl) emptyEl.style.display = seg ? 'none' : '';

  if (!seg) return;

  // Check if live segment is adjacent to the last closed segment (< 60 min gap).
  const segStart = new Date(seg.started_at).getTime() / 1000;
  const adjacent = window._lastClosedEnd && (segStart - window._lastClosedEnd) < 3600;
  // Render into the first session's inner placeholder when the live segment
  // belongs to that session (adjacent, or no closed segments at all).
  const useInner = innerEl && (adjacent || !window._lastClosedEnd);

  const segEnd = new Date(new Date(seg.started_at).getTime() + seg.duration_s * 1000);

  if (useInner) {
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
    innerEl.innerHTML = html;

    // Merge live segment totals into the first session's header.
    const cal = (window._sessionClosedCal || 0) + (seg.moving ? seg.active_calories_kcal : 0);
    const dist = (window._sessionClosedDist || 0) + (seg.moving ? seg.distance_m : 0);
    const dur = (window._sessionClosedDur || 0) + (seg.moving ? seg.duration_s : 0);

    let statsEl = document.getElementById('session-stats-0');
    let endEl = document.getElementById('session-end-0');

    // If no header exists yet (no closed segments), create one.
    if (!statsEl) {
      const segStartDate = new Date(seg.started_at);
      const headerHtml = '<div class="flex items-center justify-between mb-4">' +
        '<div class="text-sm text-gray-400">' + formatDate(segStartDate) + ' · ' + formatTime(segStartDate) + ' – <span id="session-end-0">' + formatTime(segEnd) + '</span></div>' +
        '<div id="session-stats-0" class="text-sm text-gray-500">' + cal.toFixed(1) + ' kcal · ' + (dist / 1000).toFixed(2) + ' km · ' + formatDurationLong(dur) + '</div>' +
        '</div>';
      innerEl.parentElement.insertAdjacentHTML('afterbegin', headerHtml);
    } else {
      statsEl.textContent = cal.toFixed(1) + ' kcal \u00b7 ' + (dist / 1000).toFixed(2) + ' km \u00b7 ' + formatDurationLong(dur);
      if (endEl) endEl.textContent = formatTime(segEnd);
    }
  } else if (outerEl) {
    // Non-adjacent: live segment is its own new session. Wrap it in a full
    // session card so it matches closed-session styling. Totals reflect only
    // the live segment (separate session from anything closed below it).
    const segStartDate = new Date(seg.started_at);
    const cal = seg.moving ? seg.active_calories_kcal : 0;
    const dist = seg.moving ? seg.distance_m : 0;
    const dur = seg.moving ? seg.duration_s : 0;
    let html = '<div class="bg-surface-800 rounded-xl p-5 border border-gray-800 mb-4">';
    html += '<div class="flex items-center justify-between mb-4">';
    html += '<div class="text-sm text-gray-400">' + formatDate(segStartDate) + ' · ' + formatTime(segStartDate) + ' – ' + formatTime(segEnd) + '</div>';
    html += '<div class="text-sm text-gray-500">' + cal.toFixed(1) + ' kcal \u00b7 ' + (dist / 1000).toFixed(2) + ' km \u00b7 ' + formatDurationLong(dur) + '</div>';
    html += '</div>';
    html += renderSegmentCard(seg);
    html += '</div>';
    outerEl.innerHTML = html;
  }
}

function renderSegmentCard(seg) {
  const dur = seg.duration_s;
  if (seg.moving) {
    const kcalPerH = dur > 0 ? (seg.active_calories_kcal * 3600) / dur : 0;
    const segStart = new Date(seg.started_at);
    const segEnd = new Date(segStart.getTime() + dur * 1000);
    let html = '<div class="bg-surface-900/50 rounded-lg px-4 py-2.5 border border-gray-800/50">';
    html += '<div class="segment-row text-sm">';
    if (seg.open) {
      html += '<div class="w-2.5 h-2.5 rounded-full bg-status-walking flex-shrink-0 live-blink" style="grid-column:1"></div>';
    }
    html += '<span class="text-gray-400" style="grid-column:2">' + formatTime(segStart) + '–' + formatTime(segEnd) + '</span>';
    html += '<span class="text-white font-medium" style="grid-column:3">' + formatDurationLong(dur) + '</span>';
    html += '<span class="text-gray-300" style="grid-column:4">' + (seg.distance_m / 1000).toFixed(2) + ' km</span>';
    html += '<span class="text-gray-300" style="grid-column:5">' + seg.active_calories_kcal.toFixed(1) + ' kcal</span>';
    html += '<span class="text-gray-500" style="grid-column:6">' + seg.speed_kmh.toFixed(1) + ' km/h</span>';
    html += '<span class="text-gray-600 text-xs" style="grid-column:7">' + kcalPerH.toFixed(1) + ' kcal/h</span>';
    html += '<span class="text-gray-600 text-xs" style="grid-column:8">' + seg.weight_kg.toFixed(0) + ' kg</span>';
    html += '<span class="text-gray-600 text-xs" style="grid-column:9">' + inclineLabel(seg.incline_percent) + '</span>';
    html += '</div>';
    html += '</div>';
    return html;
  } else {
    let html = '<div class="text-center text-xs text-gray-600 py-1.5">';
    html += 'idle ' + formatDurationLong(dur);
    html += '</div>';
    return html;
  }
}

function formatDurationLong(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return h + ':' + String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0');
  return String(m).padStart(2, '0') + ':' + String(s).padStart(2, '0');
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
    if (currentHistoryId) fetchHistoryClosed();
    if (currentPage === 'leaderboard') {
      fetchLeaderboard();
      // Refetch day chart only when viewing today (past dates are immutable).
      if (dayViewingToday()) fetchDay();
    }
    // Refetch profile if viewing it — updates Last 7 Days bars and live indicator.
    if (!document.getElementById('page-profile').classList.contains('hidden')) fetchProfile();
  };

  ws.onclose = () => {
    setTimeout(connect, 2000);
  };

  ws.onerror = () => ws.close();
}

// -- Init --
const currentPage = initPage();

connect();

// Leaderboard and day chart are leaderboard-page-only. Other pages don't fetch or
// poll them — saves a request per page nav and a 5s poll while on profile/history.
if (currentPage === 'leaderboard') {
  fetchLeaderboard();
  fetchDay();
  setInterval(fetchLeaderboard, LEADERBOARD_POLL_INTERVAL_MS);
}
