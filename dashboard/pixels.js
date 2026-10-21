// -- Pixel art: food ladder and walker sprite --
// Each image is a grid of characters; '.' is transparent and every other
// character is looked up in the image's palette. Rendered to SVG on load.

function pixelRects(rows, palette, dx, dy) {
  let out = '';
  rows.forEach((row, y) => {
    let x = 0;
    while (x < row.length) {
      const ch = row[x];
      let end = x + 1;
      while (end < row.length && row[end] === ch) end++;
      if (ch !== '.') {
        out += '<rect x="' + (x + dx) + '" y="' + (y + dy) + '" width="' + (end - x) + '" height="1" fill="' + palette[ch] + '"/>';
      }
      x = end;
    }
  });
  return out;
}

function pixelSvg(width, height, body) {
  return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + width + ' ' + height + '" shape-rendering="crispEdges">' + body + '</svg>';
}

// Ascending by kcal. Each food is a milestone: on the trail, in the heatmap
// tooltip and in "You Burned", a total shows the biggest food it has reached.
const FOODS = [
  { name: 'Lollipop', kcal: 11,
    palette: { o: '#5a1f2a', r: '#e84a5f', w: '#fff3e0', s: '#e8dcc0' },
    rows: [
      '................',
      '.....oooooo.....',
      '....orrwwrro....',
      '...orwwrrwwro...',
      '...owrrwwrrwo...',
      '...orrwrrwrro...',
      '...owwrwwrwwo...',
      '...orwrrrrwro...',
      '....orwwwwro....',
      '.....oooooo.....',
      '.......ss.......',
      '.......ss.......',
      '.......ss.......',
      '.......ss.......',
      '.......ss.......',
      '................',
    ] },
  { name: 'Marshmallow', kcal: 23,
    palette: { o: '#6b4a55', w: '#ffffff', v: '#ffd6e0', s: '#e8a8bb' },
    rows: [
      '................',
      '................',
      '................',
      '....oooooooo....',
      '...owwwwwwwwo...',
      '..owwwwwwwwwwo..',
      '..ovwwwwwwwwvo..',
      '..ovvvvvvvvvvo..',
      '..ovvvvvvvvvvo..',
      '..ovvvvvvvvvvo..',
      '..ovvvvvvvvvvo..',
      '..osvvvvvvvvso..',
      '...osssssssso...',
      '....oooooooo....',
      '................',
      '................',
    ] },
  { name: 'Oreo cookie', kcal: 53,
    palette: { o: '#140c0a', d: '#3b2a26', h: '#5a4540', b: '#2a1d1a', c: '#f5f0e6' },
    rows: [
      '................',
      '................',
      '................',
      '....oooooooo....',
      '..oodhddddhdoo..',
      '.odddddhhdddddo.',
      '.odhddddddddhdo.',
      '..oodddhhdddoo..',
      '.obbbbbbbbbbbbo.',
      '.occcccccccccco.',
      '.occcccccccccco.',
      '.obbbbbbbbbbbbo.',
      '..oobbbbbbbboo..',
      '....oooooooo....',
      '................',
      '................',
    ] },
  { name: 'Apple', kcal: 95,
    palette: { o: '#4a0d16', r: '#d7263d', w: '#ff8a8a', d: '#9e1b2c', s: '#5b3a1e', g: '#5cb85c' },
    rows: [
      '................',
      '........sgg.....',
      '.......s.gg.....',
      '...oooo.oooo....',
      '..orrwrrrrrrro..',
      '.orrwwrrrrrrrro.',
      '.orwwrrrrrrrrro.',
      '.orwrrrrrrrrrro.',
      '.orrrrrrrrrrrro.',
      '.orrrrrrrrrrrdo.',
      '.orrrrrrrrrrddo.',
      '..orrrrrrrrrdo..',
      '..orrrrrrrrddo..',
      '...orrrorrrdo...',
      '....ooo.oooo....',
      '................',
    ] },
  { name: 'Coca-Cola 33cl', kcal: 139,
    palette: { o: '#3a0a0c', r: '#e3242b', l: '#ff6b6b', w: '#ffffff', s: '#c0c4c8' },
    rows: [
      '................',
      '.....oooooo.....',
      '....osssssso....',
      '....oooooooo....',
      '....olrrrrro....',
      '....olwrrrro....',
      '....olwrrrro....',
      '....olrwwrro....',
      '....olrrrwwo....',
      '....owwrrrro....',
      '....olrwrrro....',
      '....olrrrrro....',
      '....olrrrrro....',
      '....oooooooo....',
      '....osssssso....',
      '.....oooooo.....',
    ] },
  { name: 'Ice cream cone', kcal: 200,
    palette: { o: '#4a2c14', p: '#ff9ec7', w: '#ffffff', c: '#fff0d6', t: '#e0a458', b: '#a86b2c' },
    rows: [
      '................',
      '......oooo......',
      '....oopppwoo....',
      '...opppppwwpo...',
      '...oppppppppo...',
      '..ooppppppppoo..',
      '..occcccccccco..',
      '...ottbtbttbo...',
      '...otbttbttto...',
      '....obttbtto....',
      '....ottbttbo....',
      '.....otbtto.....',
      '.....ottbto.....',
      '......otto......',
      '.......oo.......',
      '................',
    ] },
  { name: 'Chocolate bar', kcal: 245,
    palette: { o: '#2a170c', h: '#8b5a3c', d: '#5c3a21', r: '#7b2d8b', y: '#f2c94c' },
    rows: [
      '................',
      '................',
      '..ooooooooooooo.',
      '..ohddohddohddo.',
      '..odddodddodddo.',
      '..ooooooooooooo.',
      '..ohddohddohddo.',
      '..odddodddodddo.',
      '..ooooooooooooo.',
      '..orrrrrrrrrrro.',
      '..orryyyyyyyrro.',
      '..orrrrrrrrrrro.',
      '..orryyyyyrrrro.',
      '..orrrrrrrrrrro.',
      '..ooooooooooooo.',
      '................',
    ] },
  { name: 'Hot dog', kcal: 290,
    palette: { o: '#4a2511', b: '#e8b46a', d: '#c48a3f', s: '#c0392b', y: '#f4d03f' },
    rows: [
      '................',
      '................',
      '................',
      '................',
      '...oooooooooo...',
      '..obbbbbbbbbbo..',
      '.oossssssssssoo.',
      'ossyssyssyssysso',
      'osssssssssssssso',
      '.oobbbbbbbbbboo.',
      '..obbbbbbbbbbo..',
      '...oddddddddo...',
      '....oooooooo....',
      '................',
      '................',
      '................',
    ] },
  { name: 'Medium fries', kcal: 320,
    palette: { o: '#4a0a0a', y: '#ffd23f', f: '#e0a800', r: '#d62828', w: '#ffd23f' },
    rows: [
      '................',
      '....y..y.y..y...',
      '...yy.yf.yy.y...',
      '...yfyyfyyfyyy..',
      '...yfyyfyyfyfy..',
      '..oyyfyyfyyyfyo.',
      '..orryyfyyfyrro.',
      '..orrrrrrrrrrro.',
      '..orrrrwwwrrrro.',
      '...orrwrrrwrro..',
      '...orrrrrrrrro..',
      '...orrrrrrrrro..',
      '....orrrrrrro...',
      '....ooooooooo...',
      '................',
      '................',
    ] },
  { name: 'Big Mac', kcal: 550,
    palette: { o: '#3d2010', b: '#e09f3e', w: '#fff8e7', g: '#5cb85c', c: '#ffcc33', m: '#6b3e26' },
    rows: [
      '................',
      '................',
      '.....oooooo.....',
      '...oobbbbbboo...',
      '..obbwbbbbwbbo..',
      '.obbbbbwbbbbbbo.',
      '.obbbbbbbbbbbbo.',
      '.oggggggggggggo.',
      '.occcccccccccco.',
      '.ommmmmmmmmmmmo.',
      '.ommmmmmmmmmmmo.',
      '.obbbbbbbbbbbbo.',
      '..obbbbbbbbbbo..',
      '...oooooooooo...',
      '................',
      '................',
    ] },
  { name: 'Spaghetti bolognese', kcal: 700,
    palette: { o: '#7a8a96', p: '#e8eef2', y: '#f2d16b', l: '#d9b440', r: '#b5302a', d: '#7a1d18', g: '#5cb85c' },
    rows: [
      '................',
      '................',
      '................',
      '................',
      '......rgrr......',
      '....ylrrdryl....',
      '...yllrrdrrlly..',
      '..ylyyllrrlyyly.',
      '.pylyylyylyylyp.',
      'ppyyllyyllyyllpp',
      'oppppppppppppppo',
      '.oppppppppppppo.',
      '..oooooooooooo..',
      '................',
      '................',
      '................',
    ] },
  { name: 'Big Mac meal', kcal: 1040,
    palette: { o: '#3a0a0a', r: '#d62828', w: '#ffffff', y: '#ffd23f', f: '#e0a800', b: '#e09f3e', g: '#5cb85c', m: '#6b3e26', t: '#8b5a2b', d: '#5e3a17' },
    rows: [
      '................',
      '..w.............',
      '..w.............',
      'oooo............',
      'orro.......y.yy.',
      'owro.......yfyy.',
      'orwo.......yyfy.',
      'orro.oooo..orrro',
      'owroobbbbo.orwro',
      'orrooggggo.orwro',
      'orwoommmmo.orrro',
      'orroobbbbo.orrro',
      'oooooooooo.ooooo',
      'tttttttttttttttt',
      'dddddddddddddddd',
      '................',
    ] },
  { name: 'Big Mac Plus meal', kcal: 1200,
    palette: { o: '#3a0a0a', r: '#d62828', w: '#ffffff', y: '#ffd23f', f: '#e0a800', b: '#e09f3e', g: '#5cb85c', m: '#6b3e26', t: '#8b5a2b', d: '#5e3a17' },
    rows: [
      '..w.............',
      'oooo............',
      'orro......y.yy.y',
      'owro......yyfyyy',
      'owro......yfyyfy',
      'orwo......yyyfyy',
      'orro......orrrro',
      'orwo.oooo.orwwro',
      'owroobbbboorrrro',
      'orrooggggoorwwro',
      'orroommmmoorrrro',
      'orroobbbboorrrro',
      'oooooooooooooooo',
      'tttttttttttttttt',
      'dddddddddddddddd',
      '................',
    ] },
  { name: 'Whole pizza', kcal: 1400,
    palette: { o: '#5a2e0e', k: '#d9953f', c: '#ffd966', p: '#c0392b' },
    rows: [
      '................',
      '.....oooooo.....',
      '...ookkkkkkoo...',
      '..okkcccccckko..',
      '.okcccpccccccko.',
      '.okccpppcccpcko.',
      'okcccpccccpppcko',
      'okcccccccccpccko',
      'okcpccccccccccko',
      'okppcccpccccccko',
      'okcpccpppcccccko',
      '.okccccpcccpcko.',
      '.okcccccccpppko.',
      '..okkcccccckko..',
      '...ookkkkkkoo...',
      '.....oooooo.....',
    ] },
  { name: 'Roast chicken', kcal: 1600,
    palette: { o: '#3d1f08', b: '#b8682a', h: '#e09550', d: '#7a4015', w: '#fff6e0', p: '#e8eef2' },
    rows: [
      '................',
      '..ww........ww..',
      '..wwo......oww..',
      '...obo....obo...',
      '.....oooooo.....',
      '...oohhhhhhoo...',
      '..ohhbbbbbbbbo..',
      '.ohbbbbbbbbbbbo.',
      '.obbbbbbbbbbbbo.',
      'obbbbbbbbbbbbbdo',
      'obbbbbbbbbbbbddo',
      '.odbbbbbbbbbddo.',
      '..oddddddddddo..',
      '.pppppppppppppp.',
      '..oooooooooooo..',
      '................',
    ] },
  { name: 'Birthday cake', kcal: 3000,
    palette: { o: '#5a2a3a', f: '#ff7b00', y: '#ffd23f', c: '#4fc3f7', w: '#fff8f0', p: '#f48fb1', d: '#e0e6ea' },
    rows: [
      '................',
      '...f...f...f....',
      '...y...y...y....',
      '...c...c...c....',
      '...c...c...c....',
      '.oooooooooooooo.',
      '.owwwwwwwwwwwwo.',
      '.owpwwwpwwwpwwo.',
      '.oppppppppppppo.',
      '.oppppppppppppo.',
      '.owwwwwwwwwwwwo.',
      '.oppppppppppppo.',
      '.oppppppppppppo.',
      'oddddddddddddddo',
      '.oooooooooooooo.',
      '................',
    ] },
  { name: 'Roast turkey', kcal: 6000,
    palette: { o: '#3d1f08', b: '#a0521d', h: '#d9843a', d: '#6b3410', w: '#fff6e0', g: '#3f9b3f', p: '#e8eef2' },
    rows: [
      '................',
      '.ww..........ww.',
      '.wwo........oww.',
      '..obo......obo..',
      '....oooooooo....',
      '..oohhhhhhhhoo..',
      '.ohhbbbbbbbbbbo.',
      'ohbbbbbbbbbbbbbo',
      'obbbbbbbbbbbbbbo',
      'obbbbbbbbbbbbbdo',
      'obbbbbbbbbbbbddo',
      'odbbbbbbbbbbdddo',
      'goddddddddddddog',
      'ggppppppppppppgg',
      '.pppppppppppppp.',
      '..oooooooooooo..',
    ] },
  { name: 'Wedding cake', kcal: 12000,
    palette: { o: '#8a8070', w: '#fffdf7', p: '#f5c6d6', r: '#e84a5f', d: '#c9b27c' },
    rows: [
      '......r..r......',
      '......rrrr......',
      '.......rr.......',
      '.....oooooo.....',
      '.....owwwwo.....',
      '.....owpwwo.....',
      '....oooooooo....',
      '....owwwwwwo....',
      '....owpwpwwo....',
      '...oooooooooo...',
      '...owwwwwwwwo...',
      '...owpwpwpwpo...',
      '...owwwwwwwwo...',
      '..oooooooooooo..',
      '.dddddddddddddd.',
      '................',
    ] },
  { name: 'Cheese wheel', kcal: 25000,
    palette: { o: '#6b4a00', y: '#ffd84d', h: '#e6b800', r: '#f0b429', k: '#c98f10' },
    rows: [
      '................',
      '................',
      '................',
      '..oooooooooooo..',
      '.oyyyyyyyyyyyyo.',
      'oyyyhyyyyyyhyyyo',
      'oyyyyyyyhyyyyyyo',
      '.oyyyyyyyyyyyyo.',
      'orroooooooooorro',
      'orrrrrrrrrrrrrro',
      'orrkrrrrrrrkrrro',
      'orrrrrrkrrrrrrro',
      '.orrrrrrrrrrrro.',
      '..oooooooooooo..',
      '................',
      '................',
    ] },
  { name: 'Vending machine', kcal: 60000,
    palette: { o: '#3a0a0a', r: '#c62828', g: '#22313f', a: '#ffd23f', b: '#4fc3f7', c: '#7ed957', d: '#111111', k: '#ffd23f' },
    rows: [
      '.oooooooooooooo.',
      '.orrrrrrrrrrrro.',
      '.orgggggggrrrro.',
      '.orgabcabgrrkro.',
      '.orgggggggrrrro.',
      '.orgcabcagrrkro.',
      '.orgggggggrrrro.',
      '.orgbcabcgrrkro.',
      '.orgggggggrrrro.',
      '.orgabcabgrrrro.',
      '.orgggggggrrrro.',
      '.orrrrrrrrrrrro.',
      '.orddddddddrrro.',
      '.orrrrrrrrrrrro.',
      '.oooooooooooooo.',
      '.oo..........oo.',
    ] },
];

FOODS.forEach(f => {
  f.svg = pixelSvg(16, 16, pixelRects(f.rows, f.palette, 0, 0));
});

// The biggest food reached by `kcal`, or null below the smallest.
function foodFor(kcal) {
  let found = null;
  for (const f of FOODS) {
    if (f.kcal <= kcal) found = f;
  }
  return found;
}

function foodIcon(f, px) {
  return '<span class="pixel-icon inline-block align-middle" style="width:' + px + 'px;height:' + px + 'px" title="' + f.name + ' (' + f.kcal + ' kcal)">' + f.svg + '</span>';
}

// -- Walker sprite --
// 12x17 frames, facing right. Head, torso and legs are drawn separately so
// frames can bob and swap limbs. 'C' (shirt), 'H' (hair) and 'S' (skin) come
// from the user; 'Q' is the far leg, a shade darker than 'P'.

const WALKER_W = 12;
const WALKER_H = 17;

const WALKER_HEAD = [
  '....HHHH....',
  '...HHHHHH...',
  '...HHSSSS...',
  '...HSSSES...',
  '....SSSSS...',
  '.....SS.....',
];
const WALKER_HEAD_BLINK = [
  '....HHHH....',
  '...HHHHHH...',
  '...HHSSSS...',
  '...HSSSSS...',
  '....SSSSS...',
  '.....SS.....',
];
const WALKER_TORSO_DOWN = [
  '....CCCC....',
  '...CCCCCC...',
  '...CCCCCC...',
  '...CCCCCC...',
  '....SCCC....',
];
const WALKER_TORSO_SWING = [
  '....CCCC....',
  '...CCCCCC...',
  '..CCCCCCCC..',
  '.SCCCCCCCCS.',
  '....CCCC....',
];
const WALKER_LEGS_STRIDE = [
  '....PPPP....',
  '...QQ..PP...',
  '..QQ....PP..',
  '..QQ.....PP.',
  '.BB......BBB',
];
const WALKER_LEGS_PASS = [
  '....PPPP....',
  '....QPPP....',
  '...QQ.PP....',
  '..BB..PP....',
  '......BBB...',
];
const WALKER_LEGS_STAND = [
  '....PPPP....',
  '....QP.PP...',
  '....QP.PP...',
  '....QP.PP...',
  '....BBB.BBB.',
];

function swapLegs(rows) {
  return rows.map(r => r.replace(/[PQ]/g, c => {
    if (c === 'P') return 'Q';
    return 'P';
  }));
}

// Frames 0-3 walk cycle, 4-5 idle (breathe + blink).
const WALKER_FRAMES = [
  { head: WALKER_HEAD, torso: WALKER_TORSO_SWING, legs: WALKER_LEGS_STRIDE, dy: 1, headDy: 1 },
  { head: WALKER_HEAD, torso: WALKER_TORSO_DOWN, legs: WALKER_LEGS_PASS, dy: 0, headDy: 0 },
  { head: WALKER_HEAD, torso: WALKER_TORSO_SWING, legs: swapLegs(WALKER_LEGS_STRIDE), dy: 1, headDy: 1 },
  { head: WALKER_HEAD, torso: WALKER_TORSO_DOWN, legs: swapLegs(WALKER_LEGS_PASS), dy: 0, headDy: 0 },
  { head: WALKER_HEAD, torso: WALKER_TORSO_DOWN, legs: WALKER_LEGS_STAND, dy: 1, headDy: 1 },
  { head: WALKER_HEAD_BLINK, torso: WALKER_TORSO_DOWN, legs: WALKER_LEGS_STAND, dy: 1, headDy: 2 },
];

const WALKER_HAIR = ['#2b1d14', '#5a3825', '#a0522d', '#d8b25a', '#1c1c1c', '#8b8b8b'];
const WALKER_SKIN = ['#f5d0b0', '#e0ac69', '#c68642', '#8d5524'];

// Horizontal sprite sheet of all frames, as a CSS url() value.
function walkerSheet(shirt, seed) {
  const palette = {
    C: shirt,
    H: WALKER_HAIR[seed % WALKER_HAIR.length],
    S: WALKER_SKIN[(seed >>> 4) % WALKER_SKIN.length],
    E: '#1d1d1d',
    P: '#3c4a63',
    Q: '#2b3547',
    B: '#1d1d1d',
  };
  let body = '';
  WALKER_FRAMES.forEach((f, i) => {
    const dx = i * WALKER_W;
    body += pixelRects(f.torso, palette, dx, f.dy + 6);
    body += pixelRects(f.legs, palette, dx, f.dy + 11);
    body += pixelRects(f.head, palette, dx, f.headDy);
  });
  const svg = pixelSvg(WALKER_W * WALKER_FRAMES.length, WALKER_H, body);
  return 'url("data:image/svg+xml,' + encodeURIComponent(svg) + '")';
}
