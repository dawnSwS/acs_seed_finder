// Experimental cooperative candidate. One workgroup owns one complete seed.
// Packed terrain, born union, source snapshots and prefixes reside in LDS.
struct Config {
    width: u32, height: u32, maps: u32, capacity: u32,
    stage: u32, dirs_cap: u32, max_draws: u32, padding: u32,
}
@group(0) @binding(0) var<uniform> cfg: Config;
@group(0) @binding(1) var<storage, read> inputs: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> dirs: array<i32>;
@group(0) @binding(3) var<storage, read_write> output: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read> coefficients: array<vec4<u32>>;
var<workgroup> terrain_words: array<u32, 4608>;
var<workgroup> born: array<u32, 1152>;
var<workgroup> snapshot: array<u32, 1152>;
var<workgroup> prefix: array<u32, 1152>;
var<workgroup> rng_ring: array<u32, 56>;
var<workgroup> chunks: array<u32, 32>;
var<workgroup> dirs_len: u32;
var<workgroup> broadcast_value: i32;
var<workgroup> shared_error: u32;
var<workgroup> shared_count: u32;
var<private> sid: u32;
var<private> lane: u32;
var<private> seed_magnitude: u32;
var<private> seed_family: u32;
var<private> rng_draws: u32;
var<private> rng_queries: u32;
var<private> error_code: u32;
var<private> ring_filled: u32;
var<private> track_membership: bool;
var<private> tracked_terrain: u32;

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
const ALL: u32 = 0u;
const NO_BORN: u32 = 1u;
const CHECK: u32 = 2u;
const CHECK2: u32 = 3u;
const NO_LING: u32 = 4u;
const ONLY_SOIL: u32 = 5u;
const NEIGHBOR_ORDER: array<u32, 8> = array<u32, 8>(6u, 4u, 7u, 5u, 1u, 2u, 3u, 0u);


fn word_count() -> u32 { return (cfg.width * cfg.height + 31u) / 32u; }
fn packed_count() -> u32 { return (cfg.width * cfg.height + 7u) / 8u; }
fn valid_word(word: u32) -> u32 {
    let remaining = cfg.width * cfg.height - word * 32u;
    if remaining >= 32u { return 0xffffffffu; }
    return (1u << remaining) - 1u;
}
fn terrain_at(key: u32) -> u32 {
    return (terrain_words[key / 8u] >> ((key % 8u) * 4u)) & 15u;
}
fn set_terrain(key: u32, terrain: u32) {
    let word = key / 8u;
    let shift = (key % 8u) * 4u;
    let old = (terrain_words[word] >> shift) & 15u;
    terrain_words[word] = (terrain_words[word] & ~(15u << shift)) | (terrain << shift);
    if track_membership && old != terrain {
        let bit = 1u << (key % 32u);
        if old == tracked_terrain { snapshot[key / 32u] &= ~bit; }
        if terrain == tracked_terrain { snapshot[key / 32u] |= bit; }
    }
}
fn blocked(key: u32) -> bool { return (born[key / 32u] & (1u << (key % 32u))) != 0u; }
fn allowed_terrain(key: u32, terrain: u32, condition: u32) -> bool {
    switch condition {
        case NO_BORN: { return (terrain == SOIL || terrain == FERTILE) && !blocked(key); }
        case CHECK: { return terrain == SOIL || terrain == FERTILE || terrain == LING; }
        case CHECK2: { return !(terrain == IRON || terrain == COPPER || terrain == SILVER || terrain == BROWN || terrain == GRAY || (terrain == MARBLE && !blocked(key))); }
        case NO_LING: { return terrain != LING; }
        case ONLY_SOIL: { return terrain == SOIL; }
        default: { return true; }
    }
}
fn paint(key: u32, terrain: u32, condition: u32) {
    if allowed_terrain(key, terrain_at(key), condition) { set_terrain(key, terrain); }
}
// A packed word has eight nibbles. Detect zero nibbles independently, then
// compact bit positions 0,4,...,28 to an eight-bit membership mask.
fn packed_members(packed: u32, terrain: u32) -> u32 {
    let difference = packed ^ (terrain * 0x11111111u);
    var bits = (~(((difference & 0x77777777u) + 0x77777777u) | difference | 0x77777777u) & 0x88888888u) >> 3u;
    bits = (bits | (bits >> 3u)) & 0x03030303u;
    bits = (bits | (bits >> 6u)) & 0x000f000fu;
    return (bits | (bits >> 12u)) & 255u;
}
fn membership_word(terrain: u32, word: u32) -> u32 {
    var mask = 0u;
    for (var part = 0u; part < 4u; part += 1u) {
        let index = word * 4u + part;
        if index < packed_count() { mask |= packed_members(terrain_words[index], terrain) << (part * 8u); }
    }
    return mask & valid_word(word);
}
fn sync_error() -> u32 {
    if lane == 0u { shared_error = error_code; }
    let value = workgroupUniformLoad(&shared_error);
    workgroupBarrier();
    error_code = value;
    return value;
}
fn uniform_range(first: i32, second: i32) -> i32 {
    if lane == 0u { broadcast_value = next_range(first, second); }
    let value = workgroupUniformLoad(&broadcast_value);
    workgroupBarrier();
    return value;
}
fn snapshot_terrain(terrain: u32, exclude_origin: bool) {
    for (var word = lane; word < word_count(); word += 32u) {
        var members = membership_word(terrain, word);
        if exclude_origin && word == 0u { members &= ~1u; }
        snapshot[word] = members;
    }
    workgroupBarrier();
}

fn key_at(x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= i32(cfg.width) || y >= i32(cfg.height) { return -1; }
    return y * i32(cfg.width) + x;
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
            dirs[sid * cfg.dirs_cap + dirs_len] = key;
            dirs_len += 1u;
        }
    }
}

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

// Prefix each descending contiguous source-word chunk in parallel, then
// prefix only its 32 totals on lane zero. prefix[word] is the first RNG sample
// for that word; within-word source order is also descending.
fn source_prefix(terrain: u32, radius: i32) {
    let words = word_count();
    let low = words * (31u - lane) / 32u;
    let high = words * (32u - lane) / 32u;
    var total = 0u;
    for (var remaining = high; remaining > low; remaining -= 1u) {
        let word = remaining - 1u;
        var sources = membership_word(terrain, word);
        if word == 0u { sources &= ~1u; }
        snapshot[word] = sources;
        prefix[word] = total;
        loop {
            if sources == 0u { break; }
            let bit = 31u - countLeadingZeros(sources);
            sources &= ~(1u << bit);
            let key = word * 32u + bit;
            total += square_visits(i32(key % cfg.width), i32(key / cfg.width), radius);
        }
    }
    chunks[lane] = total;
    workgroupBarrier();
    if lane == 0u {
        var position = rng_draws;
        for (var chunk = 0u; chunk < 32u; chunk += 1u) {
            let count = chunks[chunk];
            chunks[chunk] = position;
            position += count;
        }
        shared_count = position - rng_draws;
        advance_draws(shared_count);
    }
    workgroupBarrier();
    for (var word = low; word < high; word += 1u) { prefix[word] += chunks[lane]; }
    workgroupBarrier();
}
fn source_start(key: u32, radius: i32) -> u32 {
    let word = key / 32u;
    let bit = key % 32u;
    var higher = snapshot[word] & (~((1u << bit) - 1u)) & ~(1u << bit);
    var start = prefix[word];
    if cfg.width % 32u == 0u {
        let stride = cfg.width / 32u;
        let column = word % stride;
        let y = i32(key / cfg.width);
        let height = u32(min(y + radius, i32(cfg.height) - 1) - max(y - radius, 0) + 1);
        var widths = countOneBits(higher) * u32(2 * radius + 1);
        if column == 0u {
            for (var distance = 0u; distance < u32(radius); distance += 1u) {
                widths -= ((higher >> distance) & 1u) * (u32(radius) - distance);
            }
        }
        if column + 1u == stride {
            for (var distance = 0u; distance < u32(radius); distance += 1u) {
                widths -= ((higher >> (31u - distance)) & 1u) * (u32(radius) - distance);
            }
        }
        return start + widths * height;
    }
    loop {
        if higher == 0u { break; }
        let source_bit = 31u - countLeadingZeros(higher);
        higher &= ~(1u << source_bit);
        let source = word * 32u + source_bit;
        start += square_visits(i32(source % cfg.width), i32(source / cfg.width), radius);
    }
    return start;
}
fn any_outline_source_passes(tx: i32, ty: i32, radius: i32, certain: bool, bound: u32) -> bool {
    let lx = max(tx - radius, 0);
    let hx = min(tx + radius, i32(cfg.width) - 1);
    let ly = max(ty - radius, 0);
    let hy = min(ty + radius, i32(cfg.height) - 1);
    for (var sy = hy; sy >= ly; sy -= 1) {
        let first_key = u32(sy) * cfg.width + u32(lx);
        let last_key = u32(sy) * cfg.width + u32(hx);
        for (var source_word = i32(last_key / 32u); source_word >= i32(first_key / 32u); source_word -= 1) {
            let first = u32(max(i32(first_key) - source_word * 32, 0));
            let last = u32(min(i32(last_key) - source_word * 32, 31));
            var sources = snapshot[u32(source_word)] & (0xffffffffu << first) & (0xffffffffu >> (31u - last));
            loop {
                if sources == 0u { break; }
                if certain { return true; }
                let bit = 31u - countLeadingZeros(sources);
                sources &= ~(1u << bit);
                let key = u32(source_word) * 32u + bit;
                let sx = i32(key % cfg.width);
                let position = source_start(key, radius) + spiral_rank(sx, sy, tx - sx, ty - sy);
                if raw_at(position) <= bound { return true; }
            }
        }
    }
    return false;
}
// Build one word of destination coverage in registers. This preserves LDS
// for the source prefix while skipping empty destination regions altogether.
fn outline_coverage(word: u32, radius: i32) -> u32 {
    if cfg.width % 32u == 0u {
        let stride = cfg.width / 32u;
        let row = i32(word / stride);
        let column = word % stride;
        var coverage = 0u;
        for (var sy = max(row - radius, 0); sy <= min(row + radius, i32(cfg.height) - 1); sy += 1) {
            let source_word = u32(sy) * stride + column;
            let center = snapshot[source_word];
            var previous = 0u;
            var next = 0u;
            if column > 0u { previous = snapshot[source_word - 1u]; }
            if column + 1u < stride { next = snapshot[source_word + 1u]; }
            var horizontal = center;
            for (var distance = 1u; distance <= u32(radius); distance += 1u) {
                horizontal |= (center << distance) | (center >> distance)
                    | (previous >> (32u - distance)) | (next << (32u - distance));
            }
            coverage |= horizontal;
        }
        return coverage;
    }
    var coverage = 0u;
    for (var bit = 0u; bit < 32u; bit += 1u) {
        let key = word * 32u + bit;
        if key >= cfg.width * cfg.height { break; }
        if any_outline_source_passes(i32(key % cfg.width), i32(key / cfg.width), radius, true, 0u) {
            coverage |= 1u << bit;
        }
    }
    return coverage;
}
fn outline(source: u32, destination: u32, radius: i32, level: i32, condition: u32) {
    if sync_error() != 0u { return; }
    source_prefix(source, radius);
    if sync_error() != 0u { return; }
    let visits = workgroupUniformLoad(&shared_count);
    if visits == 0u || level < 0 || (radius == 0 && source == destination) { return; }
    let certain = level >= 99;
    let probability = u32(clamp(level + 1, 1, 99));
    let bound = probability * 21474836u + (probability * 47u + 99u) / 100u - 1u;
    // One invocation owns four adjacent packed u32s for 32 destination cells.
    // Coverage and existing destination masks remove queries with no effect.
    for (var word = lane; word < word_count(); word += 32u) {
        let coverage = outline_coverage(word, radius);
        if coverage == 0u { continue; }
        for (var packed_part = 0u; packed_part < 4u; packed_part += 1u) {
            let index = word * 4u + packed_part;
            if index >= packed_count() { break; }
            var packed = terrain_words[index];
            var targets = ((coverage >> (packed_part * 8u)) & 255u) & ~packed_members(packed, destination);
            loop {
                if targets == 0u { break; }
                let part = countTrailingZeros(targets);
                targets &= targets - 1u;
                let key = index * 8u + part;
                let terrain = (packed >> (part * 4u)) & 15u;
                if !allowed_terrain(key, terrain, condition) { continue; }
                var accepted = certain;
                if !certain { accepted = any_outline_source_passes(i32(key % cfg.width), i32(key / cfg.width), radius, false, bound); }
                if accepted { packed = (packed & ~(15u << (part * 4u))) | (destination << (part * 4u)); }
            }
            terrain_words[index] = packed;
        }
    }
    workgroupBarrier();
}

fn source_word(plane: u32, word: i32, exclude_zero: bool) -> u32 {
    if word < 0 || word >= i32(word_count()) { return 0u; }
    var value = snapshot[u32(word)];
    if exclude_zero && word == 0 { value &= ~1u; }
    return value;
}

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

fn add_neighbors(counts: vec4<u32>, mask: u32) -> vec4<u32> {
    let c1 = counts.x & mask;
    let c2 = counts.y & c1;
    let c4 = counts.z & c2;
    return vec4<u32>(counts.x ^ mask, counts.y ^ c1, counts.z ^ c2, counts.w ^ c4);
}

fn optimize(terrain: u32, minimum: i32, maximum: i32, condition: u32) {
    if minimum > maximum { return; }
    if sync_error() != 0u { return; }
    snapshot_terrain(terrain, false);
    let words = word_count();
    let width = i32(cfg.width);
    for (var word = lane; word < words; word += 32u) {
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
        prefix[word] = selected & valid_word(word);
    }
    workgroupBarrier();
    for (var packed_word = lane; packed_word < packed_count(); packed_word += 32u) {
        var packed = terrain_words[packed_word];
        let pending = (prefix[packed_word / 4u] >> ((packed_word % 4u) * 8u)) & 255u;
        for (var part = 0u; part < 8u; part += 1u) {
            let key = packed_word * 8u + part;
            if key >= cfg.width * cfg.height { break; }
            let old = (packed >> (part * 4u)) & 15u;
            if (pending & (1u << part)) != 0u && allowed_terrain(key, old, condition) {
                packed = (packed & ~(15u << (part * 4u))) | (terrain << (part * 4u));
            }
        }
        terrain_words[packed_word] = packed;
    }
    workgroupBarrier();
}

fn expand_ordered(terrain: u32, level: i32, reverse: bool, condition: u32) {
    let words = word_count();
    // Reload the current word after every paint: newly created members ahead
    // of the cursor must be visited in this same pass; members behind it must
    // not be revisited. A frozen source bitmap would be semantically wrong.
    for (var offset = 0u; offset < words && error_code == 0u; offset += 1u) {
        let word = select(offset, words - 1u - offset, reverse);
        var unvisited = valid_word(word);
        loop {
            let members = snapshot[word] & unvisited;
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

fn expand(terrain: u32, level: i32, reverse: bool, condition: u32) {
    if sync_error() != 0u { return; }
    snapshot_terrain(terrain, false);
    if lane == 0u {
        track_membership = true;
        tracked_terrain = terrain;
        expand_ordered(terrain, level, reverse, condition);
        track_membership = false;
    }
    workgroupBarrier();
}
fn random_and_expand(terrain: u32, random_count: i32, passes: i32, level: i32, minimum: i32, maximum: i32, condition: u32, expansion_condition: u32) {
    if sync_error() != 0u { return; }
    if lane == 0u {
        for (var n = 0; n < max(random_count, 1) && error_code == 0u; n += 1) {
            let x = next_range(0, i32(cfg.width));
            let y = next_range(0, i32(cfg.height));
            paint(u32(y * i32(cfg.width) + x), terrain, condition);
        }
    }
    workgroupBarrier();
    for (var iteration = 0; iteration < passes; iteration += 1) {
        expand(terrain, level, (iteration % 2) != 0, expansion_condition);
    }
    if minimum > 0 { optimize(terrain, minimum, maximum, expansion_condition); }
}

fn random_line_ordered(radius: i32, size: i32, terrain: u32, condition: u32) {
    if dirs_len == 0u || error_code != 0u { return; }
    var cursor = u32(next_range(0, i32(dirs_len)));
    for (var n = 0; n < size && error_code == 0u; n += 1) {
        if cursor >= dirs_len { cursor = u32(next_range(0, i32(dirs_len))); }
        var key = dirs[sid * cfg.dirs_cap + cursor];
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

}

fn random_line(radius: i32, size: i32, terrain: u32, condition: u32) {
    if sync_error() != 0u { return; }
    let length = workgroupUniformLoad(&dirs_len);
    if length == 0u { return; }
    if lane == 0u { random_line_ordered(radius, size, terrain, condition); }
    workgroupBarrier();
    outline(terrain, terrain, radius, 4, condition);
}
fn build_base() {
    if lane == 0u {
        for (var n = 0u; n < 3u && error_code == 0u; n += 1u) { make_mine_dir(2, 3); }
    }
    if sync_error() != 0u { return; }
    for (var word = lane; word < packed_count(); word += 32u) { terrain_words[word] = 0x11111111u; }
    for (var word = lane; word < word_count(); word += 32u) { born[word] = 0u; }
    workgroupBarrier();
    if lane == 0u {
        let x = next_range(i32(cfg.width) / 10 * 4, i32(cfg.width) / 10 * 7);
        let y = next_range(i32(cfg.height) / 10 * 6, i32(cfg.height) / 10 * 7);
        let key = key_at(x, y);
        if key >= 0 { set_terrain(u32(key), FERTILE); }
    }
    workgroupBarrier();
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
        if key >= 0 { born[u32(key) / 32u] |= 1u << (u32(key) % 32u); }
        offset += 1;
    }
}

fn build_water(scale: i32) {
    if sync_error() != 0u { return; }
    // Source snapshot excludes origin, matching the born-space rule.
    source_prefix(FERTILE, 2);
    if sync_error() != 0u { return; }
    for (var word = lane; word < word_count(); word += 32u) { born[word] = snapshot[word]; }
    if lane == 0u {
        var fertile_origin = terrain_at(0u) == FERTILE;
        for (var y = 0u; y < min(cfg.height, 3u); y += 1u) {
            for (var x = 0u; x < min(cfg.width, 3u); x += 1u) {
                let key = y * cfg.width + x;
                fertile_origin = fertile_origin || (snapshot[key / 32u] & (1u << (key % 32u))) != 0u;
            }
        }
        broadcast_value = select(0, 1, fertile_origin);
    }
    let origin = workgroupUniformLoad(&broadcast_value);
    for (var word = lane; word < packed_count(); word += 32u) {
        var packed = 0x11111111u;
        if word == 0u && origin != 0 { packed = (packed & ~15u) | FERTILE; }
        terrain_words[word] = packed;
    }
    workgroupBarrier();
    if lane == 0u { for (var side = 0u; side < 4u && error_code == 0u; side += 1u) { add_born_line(side); } }
    workgroupBarrier();
    random_and_expand(FERTILE, 20, 4, 30, 5, 3, ALL, ALL);
    random_and_expand(DDEPTH, scale - 1, 2 * scale - 1, 13 + 6 * scale, 5, 3, NO_BORN, NO_BORN);
    outline(DDEPTH, DEPTH, 1, 100, CHECK);
    outline(DEPTH, DEPTH, 1, 10 + 6 * scale, NO_BORN);
    outline(DEPTH, SHALLOW, 4, 50 + 12 * scale, CHECK);
}

fn build_mine_lines(scale: i32) {
    var iteration = 0;
    loop {
        let limit = uniform_range(scale, scale + 2);
        if iteration >= limit || sync_error() != 0u { break; }
        let radius = uniform_range(0, scale);
        let length = uniform_range(5 + scale, 10 + scale);
        random_line(radius, length, IRON, NO_BORN);
        iteration += 1;
    }
    for (var mineral = COPPER; mineral <= SILVER; mineral += 1u) {
        for (var n = 0; n <= scale; n += 1) {
            let radius = uniform_range(0, 1);
            let length = uniform_range(3 + scale, 5 + scale);
            random_line(radius, length, mineral, NO_BORN);
        }
    }
    iteration = 0;
    loop {
        let limit = uniform_range(1, 3);
        if iteration >= limit || sync_error() != 0u { break; }
        let radius = uniform_range(0, 1);
        let length = uniform_range(8, 16);
        let mineral = select(MARBLE, GRAY, uniform_range(1, 3) == 1);
        random_line(radius, length, mineral, NO_BORN);
        iteration += 1;
    }
}

fn expand_mines(scale: i32) {
    var iteration = 1;
    loop {
        let limit = uniform_range(1, 3);
        if iteration >= limit || sync_error() != 0u { break; }
        let count = uniform_range(0, 3);
        random_and_expand(select(MARBLE, GRAY, iteration == 1), count, 3, 15 + 3 * scale, 0, 3, NO_BORN, NO_BORN);
        iteration += 1;
    }
    for (var mineral = IRON; mineral <= SILVER; mineral += 1u) {
        let count = uniform_range(0, scale + 1);
        let passes = uniform_range(1, scale + 1);
        let factor = uniform_range(1, 4);
        random_and_expand(mineral, count, passes, 13 + scale * factor, 0, 3, NO_BORN, NO_BORN);
    }
    let count = uniform_range(2, scale + 1);
    random_and_expand(BROWN, count, scale + 1, 13 + scale * 4, 0, 3, NO_BORN, NO_BORN);
}

fn build_mine_borders(scale: i32) {
    outline(GRAY, BROWN, 1, 50 + 12 * scale, NO_BORN);
    outline(MARBLE, BROWN, 1, 50 + 12 * scale, NO_BORN);
    for (var mineral = IRON; mineral <= SILVER; mineral += 1u) { outline(mineral, BROWN, 1, 50 + 12 * scale, NO_BORN); }
    for (var mineral = IRON; mineral <= SILVER; mineral += 1u) {
        let radius = uniform_range(1, 2);
        outline(mineral, BROWN, radius, 8 + 8 * scale, NO_BORN);
    }
    for (var mineral = BROWN; mineral <= MARBLE; mineral += 1u) {
        let radius = uniform_range(1, scale);
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
fn generate(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) local: u32) {
    sid = group.x;
    lane = local;
    if sid >= cfg.maps { return; }
    seed_magnitude = inputs[sid].y;
    seed_family = inputs[sid].z;
    error_code = 0u;
    rng_draws = 0u;
    rng_queries = 0u;
    ring_filled = 0u;
    track_membership = false;
    if lane == 0u { dirs_len = 0u; }
    workgroupBarrier();
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
    let status = sync_error();
    var count = 0u;
    if status == 0u {
        for (var word = lane; word < word_count(); word += 32u) { count += countOneBits(membership_word(LING, word)); }
    }
    chunks[lane] = count;
    workgroupBarrier();
    if lane == 0u {
        shared_count = 0u;
        for (var chunk = 0u; chunk < 32u; chunk += 1u) { shared_count += chunks[chunk]; }
    }
    workgroupBarrier();
    chunks[lane] = rng_queries;
    workgroupBarrier();
    if lane == 0u {
        var queries = 0u;
        for (var chunk = 0u; chunk < 32u; chunk += 1u) { queries += chunks[chunk]; }
        output[sid] = vec4<u32>(shared_count, error_code, rng_draws, queries);
    }
}
