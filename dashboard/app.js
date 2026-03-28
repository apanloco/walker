// -- Tab navigation --

function showTab(name) {
  document.querySelectorAll('.page').forEach(p => p.classList.add('hidden'));
  document.querySelectorAll('.nav-tab').forEach(a => a.classList.remove('active'));
  document.getElementById('page-' + name).classList.remove('hidden');
  document.getElementById('tab-' + name).classList.add('active');
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
    })
    .catch(() => {});
}

// -- Profile (Me page) --

function showProfile(id) {
  currentProfileId = id;
  fetchProfile();
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

function renderProfile(p) {
  const el = document.getElementById('profile-content');
  const days = p.last_30_days.days || [];
  const maxCal = Math.max(...days.map(d => d.calories_kcal), 0.1);

  const bars = days.map(d => {
    const pct = Math.max((d.calories_kcal / maxCal) * 100, 2);
    return `
      <div class="group relative flex-1 flex flex-col justify-end" style="height: 140px">
        <div class="bg-walker-500 hover:bg-walker-600 rounded-t transition-all duration-200 min-h-[2px]" style="height: ${pct}%"></div>
        <div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 hidden group-hover:block bg-gray-800 text-white text-xs px-3 py-1.5 rounded-lg shadow-lg whitespace-nowrap z-10">
          <div class="font-medium">${d.date}</div>
          <div class="text-gray-400">${d.calories_kcal.toFixed(1)} kcal</div>
          <div class="text-gray-400">${d.distance_km.toFixed(2)} km</div>
        </div>
      </div>`;
  }).join('');

  const emptyDays = 30 - days.length;
  const emptyBars = Array(emptyDays).fill(
    '<div class="flex-1" style="height: 140px"></div>'
  ).join('');

  el.innerHTML = `
    <!-- Header -->
    <div class="flex items-center gap-4 mb-6">
      ${p.avatar_url
        ? '<img class="w-16 h-16 rounded-full ring-2 ring-walker-500/30" src="' + p.avatar_url + '" alt="">'
        : '<div class="w-16 h-16 rounded-full bg-gray-700 flex items-center justify-center text-2xl font-bold text-gray-400">' + p.name[0].toUpperCase() + '</div>'
      }
      <div>
        <div class="text-2xl font-bold text-white">${p.name}</div>
        ${p.streak > 0 ? '<div class="text-yellow-400 text-sm font-medium mt-0.5">' + p.streak + ' day streak</div>' : ''}
      </div>
    </div>

    <!-- Stats cards -->
    <div class="grid grid-cols-3 gap-3 mb-6">
      <div class="bg-surface-800 rounded-xl p-4">
        <div class="text-3xl font-extrabold text-white">${p.last_30_days.total_calories_kcal.toFixed(1)}</div>
        <div class="text-xs text-gray-500 mt-1">kcal (30 days)</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4">
        <div class="text-3xl font-extrabold text-white">${p.last_30_days.total_distance_km.toFixed(2)}</div>
        <div class="text-xs text-gray-500 mt-1">km (30 days)</div>
      </div>
      <div class="bg-surface-800 rounded-xl p-4">
        <div class="text-3xl font-extrabold text-white">${formatDuration(p.last_30_days.total_active_secs)}</div>
        <div class="text-xs text-gray-500 mt-1">active (30 days)</div>
      </div>
    </div>

    <!-- Chart -->
    <div class="bg-surface-800 rounded-xl p-5">
      <h3 class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-4">Last 30 Days</h3>
      <div class="flex items-end gap-[2px]">
        ${emptyBars}${bars}
      </div>
    </div>
  `;
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

  ws.onmessage = () => {
    fetchLeaderboard();
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
