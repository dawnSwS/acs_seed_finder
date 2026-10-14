// Exact ordered map generation with maintained terrain membership bitmaps.
// Every seed owns one SoA lane. Frozen outline snapshots and in-place expansion
// are distinct operations; bitmap iteration preserves the original visit order.
struct Config {
    width: u32,
    height: u32,
    maps: u32,
    capacity: u32,
    stage: u32,
    dirs_cap: u32,
    max_draws: u32,
    padding: u32,
}
@group(0) @binding(0) var<uniform> cfg: Config;
@group(0) @binding(1) var<storage, read> inputs: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> cells: array<u32>;
@group(0) @binding(3) var<storage, read_write> dirs: array<i32>;
@group(0) @binding(4) var<storage, read_write> output: array<vec4<u32>>;
@group(0) @binding(5) var<storage, read> coefficients: array<vec4<u32>>;
// Planes 1..15 are current terrain membership, 16 frozen outline sources,
// and 0 temporary pending optimization targets. Cell flags stay in cells.
@group(0) @binding(6) var<storage, read_write> bitmaps: array<u32>;

var<private> sid: u32;
var<private> seed_magnitude: u32;
var<private> seed_family: u32;
var<private> rng_draws: u32;
var<private> rng_queries: u32;
var<private> error_code: u32;
var<private> dirs_len: u32;

const SOIL: u32 = 1u;
const DEPTH: u32 = 2u;
const DDEPTH: u32 = 3u;
const SHALLOW: u32 = 4u;
const MUD: u32 = 5u;
const FERTILE: u32 = 6u;
const LING: u32 = 7u;
const WET: u32 = 8u;
const STONE: u32 = 9u;
const BROWN: u32 = 10u;
const GRAY: u32 = 11u;
const MARBLE: u32 = 12u;
const IRON: u32 = 13u;
const COPPER: u32 = 14u;
const SILVER: u32 = 15u;
const BORN_SPACE: u32 = 0x100u;
const BORN_LINE: u32 = 0x200u;
const SNAPSHOT_PLANE: u32 = 16u;
const PENDING_PLANE: u32 = 0u;
const ALL: u32 = 0u;
const NO_BORN: u32 = 1u;
const CHECK: u32 = 2u;
const CHECK2: u32 = 3u;
const NO_LING: u32 = 4u;
const ONLY_SOIL: u32 = 5u;
const NEIGHBOR_ORDER: array<u32, 8> = array<u32, 8>(6u, 4u, 7u, 5u, 1u, 2u, 3u, 0u);

fn cell_index(key: u32) -> u32 { return key * cfg.capacity + sid; }
fn word_count() -> u32 { return (cfg.width * cfg.height + 31u) / 32u; }
fn bitmap_index(plane: u32, word: u32) -> u32 {
    return (plane * word_count() + word) * cfg.capacity + sid;
}
fn bitmap_at(plane: u32, word: u32) -> u32 {
    return bitmaps[bitmap_index(plane, word)];
}
fn valid_word(word: u32) -> u32 {
    let remaining = cfg.width * cfg.height - word * 32u;
    if remaining >= 32u { return 0xffffffffu; }
    return (1u << remaining) - 1u;
}
fn terrain_at(key: u32) -> u32 { return cells[cell_index(key)] & 255u; }
fn set_terrain(key: u32, terrain: u32) {
    let pos = cell_index(key);
    let old = cells[pos] & 255u;
    if old == terrain { return; }
    let word = key / 32u;
    let bit = 1u << (key % 32u);
    bitmaps[bitmap_index(old, word)] &= ~bit;
    bitmaps[bitmap_index(terrain, word)] |= bit;
    cells[pos] = (cells[pos] & ~255u) | terrain;
}
fn key_at(x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= i32(cfg.width) || y >= i32(cfg.height) { return -1; }
    return y * i32(cfg.width) + x;
}
fn allowed(key: u32, condition: u32) -> bool {
    let value = cells[cell_index(key)];
    let terrain = value & 255u;
    switch condition {
        case NO_BORN: { return (terrain == SOIL || terrain == FERTILE) && (value & (BORN_SPACE | BORN_LINE)) == 0u; }
        case CHECK: { return terrain == SOIL || terrain == FERTILE || terrain == LING; }
        case CHECK2: {
            return !(terrain == IRON || terrain == COPPER || terrain == SILVER || terrain == BROWN || terrain == GRAY || (terrain == MARBLE && (value & (BORN_SPACE | BORN_LINE)) == 0u));
        }
        case NO_LING: { return terrain != LING; }
        case ONLY_SOIL: { return terrain == SOIL; }
        default: { return true; }
    }
}
fn paint(key: u32, terrain: u32, condition: u32) {
    if allowed(key, condition) { set_terrain(key, terrain); }
}
fn horizontal(key: i32, delta: i32) -> i32 {
    let other = key + delta;
    if other >= 0 && other < i32(cfg.width * cfg.height) && key / i32(cfg.width) == other / i32(cfg.width) { return other; }
    return -1;
}
fn vertical(key: i32, delta: i32) -> i32 {
    let other = key + delta;
    if other > 0 && other < i32(cfg.width * cfg.height) { return other; }
    return -1;
}
fn neighbor(key: i32, direction: u32) -> i32 {
    switch direction {
        case 0u: { return vertical(key, i32(cfg.width)); }
        case 1u: { return vertical(key, -i32(cfg.width)); }
        case 2u: { return horizontal(key, -1); }
        case 3u: { return horizontal(key, 1); }
        default: {
            let left = direction == 4u || direction == 6u;
            let upper = direction == 4u || direction == 5u;
            let side = horizontal(key, select(1, -1, left));
            if side < 0 { return -1; }
            return vertical(side, select(i32(cfg.width), -i32(cfg.width), upper));
        }
    }
}

fn make_mine_dir(fx: i32, fy: i32) {
    var x = 0;
    var y = next_range(0, i32(cfg.height));
    let original_y = y;
    let discarded = next_range(0, i32(cfg.height));
    while x < i32(cfg.width) && error_code == 0u {
        x += select(1, -1, next_range(0, 100) < 10);
        if original_y > i32(cfg.height) / 2 {
            y += next_range(-fy, fx);
        } else {
            y += next_range(-fx, fy);
        }
        let key = key_at(x, y);
        if key > 0 {
            if dirs_len >= cfg.dirs_cap { error_code = 2u; return; }
            dirs[dirs_len * cfg.capacity + sid] = key;
            dirs_len += 1u;
        }
    }
}

// Source membership is frozen before any write. Offset order is the exact
// center/up/left/down/right expanding spiral from get_base_around_50().
// Every in-bounds visit consumes a draw even when its destination is forbidden.
fn outline_ordered(source: u32, destination: u32, radius: i32, level: i32, condition: u32) {
    if error_code != 0u { return; }
    let words = word_count();
    // Source identity is frozen once, even when source == destination.
    for (var word = 0u; word < words; word += 1u) {
        bitmaps[bitmap_index(SNAPSHOT_PLANE, word)] = bitmap_at(source, word);
    }
    let visits = u32((2 * radius + 1) * (2 * radius + 1));
    for (var remaining = words; remaining > 0u && error_code == 0u; remaining -= 1u) {
        let word = remaining - 1u;
        var sources = bitmap_at(SNAPSHOT_PLANE, word);
        if word == 0u { sources &= ~1u; } // key 0 is never an outline source.
        loop {
            if sources == 0u || error_code != 0u { break; }
            let bit = 31u - countLeadingZeros(sources);
            sources &= ~(1u << bit);
            let center = word * 32u + bit;
            let cx = i32(center % cfg.width);
            let cy = i32(center / cfg.width);
            if radius == 0 && source == destination {
                // The sole write can only write its existing value.
                advance_draws(1u);
                continue;
            }
            let certain = level >= 99;
            if certain {
                // The exact number of in-bounds spiral visits is the clipped
                // square area. These draws cannot affect any write decision.
                let columns = min(cx + radius, i32(cfg.width) - 1) - max(cx - radius, 0) + 1;
                let rows = min(cy + radius, i32(cfg.height) - 1) - max(cy - radius, 0) + 1;
                advance_draws(u32(columns * rows));
            }
            var dx = 0;
            var dy = 0;
            var direction = 1u;
            var side_length = 1u;
            var side_used = 0u;
            var sides_used = 0u;
            for (var visit = 0u; visit < visits && error_code == 0u; visit += 1u) {
                let target_key = key_at(cx + dx, cy + dy);
                if target_key >= 0 {
                    // Do not evaluate next_range when the draw was skipped.
                    var accepted = certain;
                    if !certain { accepted = next_range(0, 100) <= level; }
                    if accepted { paint(u32(target_key), destination, condition); }
                }
                switch direction {
                    case 0u: { dx += 1; }
                    case 1u: { dy -= 1; }
                    case 2u: { dx -= 1; }
                    default: { dy += 1; }
                }
                side_used += 1u;
                if side_used == side_length {
                    side_used = 0u;
                    direction = (direction + 1u) % 4u;
                    sides_used += 1u;
                    if sides_used == 2u { sides_used = 0u; side_length += 1u; }
                }
            }
        }
    }
}

// Inclusive clipped segment lengths preserve the original spiral's in-bounds
// random rank. A ring d visits top, left, bottom, right in that order.
fn horizontal_visits(cx: i32, cy: i32, y: i32, first: i32, last: i32) -> u32 {
    if cy + y < 0 || cy + y >= i32(cfg.height) || first > last { return 0u; }
    return u32(max(0, min(cx + last, i32(cfg.width) - 1) - max(cx + first, 0) + 1));
}
fn vertical_visits(cx: i32, cy: i32, x: i32, first: i32, last: i32) -> u32 {
    if cx + x < 0 || cx + x >= i32(cfg.width) || first > last { return 0u; }
    return u32(max(0, min(cy + last, i32(cfg.height) - 1) - max(cy + first, 0) + 1));
}
fn square_visits(cx: i32, cy: i32, radius: i32) -> u32 {
    let columns = min(cx + radius, i32(cfg.width) - 1) - max(cx - radius, 0) + 1;
    let rows = min(cy + radius, i32(cfg.height) - 1) - max(cy - radius, 0) + 1;
    return u32(columns * rows);
}
fn spiral_rank(cx: i32, cy: i32, dx: i32, dy: i32) -> u32 {
    let distance = max(abs(dx), abs(dy));
    if distance == 0 { return 0u; }
    var rank = square_visits(cx, cy, distance - 1);
    if dy == -distance && dx < distance {
        return rank + horizontal_visits(cx, cy, -distance, dx + 1, distance - 1);
    }
    rank += horizontal_visits(cx, cy, -distance, -distance, distance - 1);
    if dx == -distance {
        return rank + vertical_visits(cx, cy, -distance, -distance + 1, dy - 1);
    }
    rank += vertical_visits(cx, cy, -distance, -distance + 1, distance);
    if dy == distance {
        return rank + horizontal_visits(cx, cy, distance, -distance + 1, dx - 1);
    }
    rank += horizontal_visits(cx, cy, distance, -distance + 1, distance);
    return rank + vertical_visits(cx, cy, distance, dy + 1, distance - 1);
}
fn any_outline_source_passes(tx: i32, ty: i32, radius: i32, bound: u32) -> bool {
    let stride = cfg.width / 32u;
    let lx = max(tx - radius, 0);
    let hx = min(tx + radius, i32(cfg.width) - 1);
    let ly = max(ty - radius, 0);
    let hy = min(ty + radius, i32(cfg.height) - 1);
    for (var sy = hy; sy >= ly; sy -= 1) {
        for (var column = hx / 32; column >= lx / 32; column -= 1) {
            let first = u32(max(lx - column * 32, 0));
            let last = u32(min(hx - column * 32, 31));
            var sources = bitmap_at(SNAPSHOT_PLANE, u32(sy) * stride + u32(column))
                & (0xffffffffu << first) & (0xffffffffu >> (31u - last));
            loop {
                if sources == 0u { break; }
                let bit = 31u - countLeadingZeros(sources);
                sources &= ~(1u << bit);
                let sx = column * 32 + i32(bit);
                let key = u32(sy) * cfg.width + u32(sx);
                let position = (cells[cell_index(key)] >> 10u) + spiral_rank(sx, sy, tx - sx, ty - sy);
                if raw_at(position) <= bound { return true; }
            }
        }
    }
    return false;
}

fn outline_indexed(source: u32, destination: u32, radius: i32, level: i32, condition: u32) {
    let words = word_count();
    let stride = cfg.width / 32u;
    var position = rng_draws;
    // The low ten bits retain terrain and born flags. Each frozen source gets
    // its exact first logical draw in the unused high bits (max_draws < 2^21).
    for (var remaining = words; remaining > 0u; remaining -= 1u) {
        let word = remaining - 1u;
        var sources = bitmap_at(source, word);
        if word == 0u { sources &= ~1u; }
        bitmaps[bitmap_index(SNAPSHOT_PLANE, word)] = sources;
        loop {
            if sources == 0u { break; }
            let bit = 31u - countLeadingZeros(sources);
            sources &= ~(1u << bit);
            let key = word * 32u + bit;
            let x = i32(key % cfg.width);
            let y = i32(key / cfg.width);
            let pos = cell_index(key);
            cells[pos] = (cells[pos] & 1023u) | (position << 10u);
            position += square_visits(x, y, radius);
        }
    }
    advance_draws(position - rng_draws);
    if error_code != 0u || level < 0 { return; }
    if radius == 0 && source == destination { return; }
    // Separable square dilation costs O(words*radius), not O(sources*radius²).
    // This path is used only for rows whose lengths are multiples of 32.
    for (var word = 0u; word < words; word += 1u) {
        let column = word % stride;
        let center = bitmap_at(SNAPSHOT_PLANE, word);
        var previous = 0u;
        var next = 0u;
        if column > 0u { previous = bitmap_at(SNAPSHOT_PLANE, word - 1u); }
        if column + 1u < stride { next = bitmap_at(SNAPSHOT_PLANE, word + 1u); }
        var mask = center;
        for (var distance = 1u; distance <= u32(radius); distance += 1u) {
            mask |= (center << distance) | (center >> distance)
                | (previous >> (32u - distance)) | (next << (32u - distance));
        }
        bitmaps[bitmap_index(PENDING_PLANE, word)] = mask;
    }
    let certain = level >= 99;
    // ceil((level+1)*M/100)-1 without a 64-bit product, for 0 <= level < 99.
    let probability = u32(clamp(level + 1, 1, 99));
    let bound = probability * 21474836u + (probability * 47u + 99u) / 100u - 1u;
    for (var word = 0u; word < words; word += 1u) {
        let row = i32(word / stride);
        let column = word % stride;
        var covered = 0u;
        for (var sy = max(row - radius, 0); sy <= min(row + radius, i32(cfg.height) - 1); sy += 1) {
            covered |= bitmap_at(PENDING_PLANE, u32(sy) * stride + column);
        }
        var targets = covered & ~bitmap_at(destination, word);
        loop {
            if targets == 0u { break; }
            let bit = countTrailingZeros(targets);
            targets &= targets - 1u;
            let key = word * 32u + bit;
            if !allowed(key, condition) { continue; }
            var accepted = certain;
            if !certain {
                accepted = any_outline_source_passes(i32(key % cfg.width), row, radius, bound);
            }
            if accepted { set_terrain(key, destination); }
        }
    }
}

fn outline(source: u32, destination: u32, radius: i32, level: i32, condition: u32) {
    if error_code != 0u { return; }
    if cfg.width % 32u == 0u && radius >= 0 && radius <= 5 {
        outline_indexed(source, destination, radius, level, condition);
    } else {
        outline_ordered(source, destination, radius, level, condition);
    }
}

fn source_word(plane: u32, word: i32, exclude_zero: bool) -> u32 {
    if word < 0 || word >= i32(word_count()) { return 0u; }
    var value = bitmap_at(plane, u32(word));
    if exclude_zero && word == 0 { value &= ~1u; }
    return value;
}
// At each target bit k, return source membership at k+delta. Vertical and
// diagonal neighbor helpers reject key 0, unlike horizontal neighbors.
fn shifted_source(plane: u32, word: u32, delta: i32, exclude_zero: bool) -> u32 {
    let start = i32(word * 32u) + delta;
    let low_word = start >> 5u;
    let shift = bitcast<u32>(start) & 31u;
    let low = source_word(plane, low_word, exclude_zero);
    if shift == 0u { return low; }
    return (low >> shift) | (source_word(plane, low_word + 1, exclude_zero) << (32u - shift));
}
fn horizontal_mask(word: u32, left: bool) -> u32 {
    let start_x = (word * 32u) % cfg.width;
    let forbidden_x = select(cfg.width - 1u, 0u, left);
    var offset = (forbidden_x + cfg.width - start_x) % cfg.width;
    var mask = valid_word(word);
    loop {
        if offset >= 32u { break; }
        mask &= ~(1u << offset);
        offset += cfg.width;
    }
    return mask;
}
// Four independent bitplanes hold 32 exact counts in [0,8].
fn add_neighbors(counts: vec4<u32>, mask: u32) -> vec4<u32> {
    let c1 = counts.x & mask;
    let c2 = counts.y & c1;
    let c4 = counts.z & c2;
    return vec4<u32>(counts.x ^ mask, counts.y ^ c1, counts.z ^ c2, counts.w ^ c4);
}
fn optimize(terrain: u32, minimum: i32, maximum: i32, condition: u32) {
    if minimum > maximum || error_code != 0u { return; }
    let words = word_count();
    let width = i32(cfg.width);
    for (var word = 0u; word < words; word += 1u) {
        let left = horizontal_mask(word, true);
        let right = horizontal_mask(word, false);
        var counts = vec4<u32>(0u);
        counts = add_neighbors(counts, shifted_source(terrain, word, width, true));
        counts = add_neighbors(counts, shifted_source(terrain, word, -width, true));
        counts = add_neighbors(counts, shifted_source(terrain, word, -1, false) & left);
        counts = add_neighbors(counts, shifted_source(terrain, word, 1, false) & right);
        counts = add_neighbors(counts, shifted_source(terrain, word, -width - 1, true) & left);
        counts = add_neighbors(counts, shifted_source(terrain, word, -width + 1, true) & right);
        counts = add_neighbors(counts, shifted_source(terrain, word, width - 1, true) & left);
        counts = add_neighbors(counts, shifted_source(terrain, word, width + 1, true) & right);
        var selected = 0u;
        for (var value = max(minimum, 0); value <= min(maximum, 8); value += 1) {
            var equal = select(~counts.x, counts.x, (value & 1) != 0);
            equal &= select(~counts.y, counts.y, (value & 2) != 0);
            equal &= select(~counts.z, counts.z, (value & 4) != 0);
            equal &= select(~counts.w, counts.w, (value & 8) != 0);
            selected |= equal;
        }
        bitmaps[bitmap_index(PENDING_PLANE, word)] = selected & valid_word(word);
    }
    // All neighbor counts were frozen before writes. The allowed predicate
    // reads only its own destination, which has not been changed earlier here.
    for (var word = 0u; word < words; word += 1u) {
        var pending = bitmap_at(PENDING_PLANE, word);
        loop {
            if pending == 0u { break; }
            let bit = countTrailingZeros(pending);
            pending &= pending - 1u;
            paint(word * 32u + bit, terrain, condition);
        }
    }
}

fn expand(terrain: u32, level: i32, reverse: bool, condition: u32) {
    let words = word_count();
    // Reload the current word after every paint: newly created members ahead
    // of the cursor must be visited in this same pass; members behind it must
    // not be revisited. A frozen source bitmap would be semantically wrong.
    for (var offset = 0u; offset < words && error_code == 0u; offset += 1u) {
        let word = select(offset, words - 1u - offset, reverse);
        var unvisited = valid_word(word);
        loop {
            let members = bitmap_at(terrain, word) & unvisited;
            if members == 0u || error_code != 0u { break; }
            var bit = countTrailingZeros(members);
            if reverse { bit = 31u - countLeadingZeros(members); }
            let key = word * 32u + bit;
            if reverse {
                unvisited &= (1u << bit) - 1u;
            } else {
                unvisited &= ~((1u << bit) - 1u) & ~(1u << bit);
            }
            if next_range(0, 100) > level { continue; }
            for (var n = 0u; n < 8u; n += 1u) {
                let other = neighbor(i32(key), NEIGHBOR_ORDER[n]);
                if other >= 0 { paint(u32(other), terrain, condition); }
            }
        }
    }
}
fn random_and_expand(terrain: u32, random_count: i32, passes: i32, level: i32, minimum: i32, maximum: i32, condition: u32, expansion_condition: u32) {
    if error_code != 0u { return; }
    for (var n = 0; n < max(random_count, 1) && error_code == 0u; n += 1) {
        let x = next_range(0, i32(cfg.width));
        let y = next_range(0, i32(cfg.height));
        paint(u32(y * i32(cfg.width) + x), terrain, condition);
    }
    for (var iteration = 0; iteration < passes && error_code == 0u; iteration += 1) {
        expand(terrain, level, (iteration % 2) != 0, expansion_condition);
    }
    if minimum > 0 { optimize(terrain, minimum, maximum, expansion_condition); }
}

fn random_line(radius: i32, size: i32, terrain: u32, condition: u32) {
    if dirs_len == 0u || error_code != 0u { return; }
    var cursor = u32(next_range(0, i32(dirs_len)));
    for (var n = 0; n < size && error_code == 0u; n += 1) {
        if cursor >= dirs_len { cursor = u32(next_range(0, i32(dirs_len))); }
        var key = dirs[cursor * cfg.capacity + sid];
        cursor += 1u;
        var candidates: array<i32, 8>;
        var candidate_count = 0u;
        for (var d = 0u; d < 8u; d += 1u) {
            let other = neighbor(key, NEIGHBOR_ORDER[d]);
            if other >= 0 { candidates[candidate_count] = other; candidate_count += 1u; }
        }
        if candidate_count > 0u { key = candidates[u32(next_range(0, i32(candidate_count)))]; }
        if key >= 0 && key < i32(cfg.width * cfg.height) { paint(u32(key), terrain, condition); }
    }
    outline(terrain, terrain, radius, 4, condition);
}

fn build_base() {
    for (var n = 0u; n < 3u && error_code == 0u; n += 1u) { make_mine_dir(2, 3); }
    if error_code != 0u { return; }
    for (var key = 0u; key < cfg.width * cfg.height; key += 1u) { cells[cell_index(key)] = SOIL; }
    for (var plane = 0u; plane < 17u; plane += 1u) {
        for (var word = 0u; word < word_count(); word += 1u) {
            bitmaps[bitmap_index(plane, word)] = select(0u, valid_word(word), plane == SOIL);
        }
    }
    let x = next_range(i32(cfg.width) / 10 * 4, i32(cfg.width) / 10 * 7);
    let y = next_range(i32(cfg.height) / 10 * 6, i32(cfg.height) / 10 * 7);
    let key = key_at(x, y);
    if key >= 0 { set_terrain(u32(key), FERTILE); }
    outline(FERTILE, FERTILE, 2, 100, ALL);
    outline(FERTILE, FERTILE, 5, 20, ALL);
}

fn add_born_line(side: u32) {
    let start = next_range(i32(f32(cfg.width) * 0.3), i32(f32(cfg.width) * 0.6));
    var offset = 0;
    loop {
        let limit = next_range(5, 15);
        if offset >= limit || error_code != 0u { break; }
        var key = -1;
        switch side {
            case 0u: { key = key_at(0, start + offset); }
            case 1u: { key = key_at(i32(cfg.width) - 1, start + offset); }
            case 2u: { key = key_at(start + offset, 0); }
            default: { key = key_at(start + offset, i32(cfg.height) - 1); }
        }
        if key >= 0 { cells[cell_index(u32(key))] |= BORN_LINE; }
        offset += 1;
    }
}
fn build_water(scale: i32) {
    if error_code != 0u { return; }
    for (var key = 1u; key < cfg.width * cfg.height; key += 1u) {
        if terrain_at(key) == FERTILE { cells[cell_index(key)] |= BORN_SPACE; }
    }
    // Only SOIL/FERTILE exist here. The certain radius-two dilation is
    // followed by clearing every fertile key > 0; its only surviving map
    // effect can be origin becoming fertile. Born membership was frozen above.
    var origin_fertile = terrain_at(0u) == FERTILE;
    var skipped = 0u;
    for (var word = 0u; word < word_count(); word += 1u) {
        var sources = bitmap_at(FERTILE, word);
        if word == 0u { sources &= ~1u; }
        loop {
            if sources == 0u { break; }
            let bit = countTrailingZeros(sources);
            sources &= sources - 1u;
            let key = word * 32u + bit;
            let x = i32(key % cfg.width);
            let y = i32(key / cfg.width);
            let columns = min(x + 2, i32(cfg.width) - 1) - max(x - 2, 0) + 1;
            let rows = min(y + 2, i32(cfg.height) - 1) - max(y - 2, 0) + 1;
            skipped += u32(columns * rows);
            origin_fertile = origin_fertile || (x <= 2 && y <= 2);
            set_terrain(key, SOIL);
        }
    }
    advance_draws(skipped);
    if origin_fertile { set_terrain(0u, FERTILE); }
    for (var side = 0u; side < 4u && error_code == 0u; side += 1u) { add_born_line(side); }
    random_and_expand(FERTILE, 20, 4, 30, 5, 3, ALL, ALL);
    random_and_expand(DDEPTH, scale - 1, 2 * scale - 1, 13 + 6 * scale, 5, 3, NO_BORN, NO_BORN);
    outline(DDEPTH, DEPTH, 1, 100, CHECK);
    outline(DEPTH, DEPTH, 1, 10 + 6 * scale, NO_BORN);
    outline(DEPTH, SHALLOW, 4, 50 + 12 * scale, CHECK);
}

fn build_mine_lines(scale: i32) {
    var iteration = 0;
    loop {
        let limit = next_range(scale, scale + 2);
        if iteration >= limit || error_code != 0u { break; }
        let radius = next_range(0, scale);
        let length = next_range(5 + scale, 10 + scale);
        random_line(radius, length, IRON, NO_BORN);
        iteration += 1;
    }
    for (var mineral = COPPER; mineral <= SILVER && error_code == 0u; mineral += 1u) {
        for (var n = 0; n <= scale && error_code == 0u; n += 1) {
            let radius = next_range(0, 1);
            let length = next_range(3 + scale, 5 + scale);
            random_line(radius, length, mineral, NO_BORN);
        }
    }
    iteration = 0;
    loop {
        let limit = next_range(1, 3);
        if iteration >= limit || error_code != 0u { break; }
        let radius = next_range(0, 1);
        let length = next_range(8, 16);
        let mineral = select(MARBLE, GRAY, next_range(1, 3) == 1);
        random_line(radius, length, mineral, NO_BORN);
        iteration += 1;
    }
}
fn expand_mines(scale: i32) {
    var iteration = 1;
    loop {
        let limit = next_range(1, 3);
        if iteration >= limit || error_code != 0u { break; }
        let count = next_range(0, 3);
        random_and_expand(select(MARBLE, GRAY, iteration == 1), count, 3, 15 + 3 * scale, 0, 3, NO_BORN, NO_BORN);
        iteration += 1;
    }
    for (var mineral = IRON; mineral <= SILVER && error_code == 0u; mineral += 1u) {
        let count = next_range(0, scale + 1);
        let passes = next_range(1, scale + 1);
        let factor = next_range(1, 4);
        random_and_expand(mineral, count, passes, 13 + scale * factor, 0, 3, NO_BORN, NO_BORN);
    }
    let count = next_range(2, scale + 1);
    random_and_expand(BROWN, count, scale + 1, 13 + scale * 4, 0, 3, NO_BORN, NO_BORN);
}
fn build_mine_borders(scale: i32) {
    outline(GRAY, BROWN, 1, 50 + 12 * scale, NO_BORN);
    outline(MARBLE, BROWN, 1, 50 + 12 * scale, NO_BORN);
    for (var mineral = IRON; mineral <= SILVER && error_code == 0u; mineral += 1u) { outline(mineral, BROWN, 1, 50 + 12 * scale, NO_BORN); }
    for (var mineral = IRON; mineral <= SILVER && error_code == 0u; mineral += 1u) {
        let radius = next_range(1, 2);
        outline(mineral, BROWN, radius, 8 + 8 * scale, NO_BORN);
    }
    for (var mineral = BROWN; mineral <= MARBLE && error_code == 0u; mineral += 1u) {
        let radius = next_range(1, scale);
        outline(mineral, BROWN, radius, 8 + 8 * scale, NO_BORN);
    }
}
fn build_mines(scale: i32) {
    random_and_expand(SHALLOW, 3, 3, 20, 5, 3, NO_BORN, NO_BORN);
    outline(SHALLOW, MUD, 4, 90, CHECK);
    build_mine_lines(scale);
    expand_mines(scale);
    build_mine_borders(scale);
}
fn build_stone() {
    outline(BROWN, STONE, 1, 30, CHECK2);
    outline(IRON, STONE, 1, 30, CHECK2);
    outline(SILVER, STONE, 1, 30, CHECK2);
    outline(COPPER, STONE, 1, 30, CHECK2);
    outline(BROWN, STONE, 1, 30, CHECK2);
    outline(STONE, STONE, 1, 5, CHECK);
}
fn postprocess_ling() {
    outline(LING, WET, 1, 100, NO_LING);
    outline(WET, WET, 1, 90, NO_LING);
    outline(WET, WET, 1, 5, CHECK);
    optimize(WET, 2, 9, CHECK);
    outline(MUD, MUD, 1, 10, CHECK);
    optimize(MUD, 2, 9, CHECK);
    random_and_expand(MUD, 5, 3, 20, 5, 3, CHECK, CHECK);
    random_and_expand(FERTILE, 5, 3, 20, 5, 3, CHECK, CHECK);
    random_and_expand(WET, 5, 3, 20, 5, 3, CHECK, CHECK);
}

@compute @workgroup_size(32)
fn generate(@builtin(global_invocation_id) invocation: vec3<u32>) {
    sid = invocation.x;
    if sid >= cfg.maps { return; }
    error_code = 0u;
    seed_magnitude = inputs[sid].y;
    seed_family = inputs[sid].z;
    rng_draws = 0u;
    rng_queries = 0u;
    dirs_len = 0u;
    let scale = max(1, i32(cfg.width) / 64);
    build_base();
    optimize(FERTILE, 2, 4, ALL);
    build_water(scale);
    optimize(SHALLOW, 2, 6, CHECK);
    build_mines(scale);
    optimize(BROWN, 2, 9, NO_BORN);
    build_stone();
    optimize(STONE, 2, 9, CHECK);
    random_and_expand(LING, scale, 6, 33, 5, 3, CHECK, CHECK);
    if cfg.stage == 1u { postprocess_ling(); }
    var count = 0u;
    if error_code == 0u {
        for (var word = 0u; word < word_count(); word += 1u) { count += countOneBits(bitmap_at(LING, word)); }
    }
    output[sid] = vec4<u32>(count, error_code, rng_draws, rng_queries);
}
