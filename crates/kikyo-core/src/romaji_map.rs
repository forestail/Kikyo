use std::collections::HashMap;

lazy_static::lazy_static! {
    static ref KANA_ROMAJI_MAP: HashMap<char, &'static str> = {
        let mut m = HashMap::new();
        // Hiragana
        m.insert('あ', "a"); m.insert('い', "i"); m.insert('う', "u"); m.insert('え', "e"); m.insert('お', "o");
        m.insert('か', "ka"); m.insert('き', "ki"); m.insert('く', "ku"); m.insert('け', "ke"); m.insert('こ', "ko");
        m.insert('さ', "sa"); m.insert('し', "shi"); m.insert('す', "su"); m.insert('せ', "se"); m.insert('そ', "so");
        m.insert('た', "ta"); m.insert('ち', "chi"); m.insert('つ', "tsu"); m.insert('て', "te"); m.insert('と', "to");
        m.insert('な', "na"); m.insert('に', "ni"); m.insert('ぬ', "nu"); m.insert('ね', "ne"); m.insert('の', "no");
        m.insert('は', "ha"); m.insert('ひ', "hi"); m.insert('ふ', "hu"); m.insert('へ', "he"); m.insert('ほ', "ho");
        m.insert('ま', "ma"); m.insert('み', "mi"); m.insert('む', "mu"); m.insert('め', "me"); m.insert('も', "mo");
        m.insert('や', "ya"); m.insert('ゆ', "yu"); m.insert('よ', "yo");
        m.insert('ら', "ra"); m.insert('り', "ri"); m.insert('る', "ru"); m.insert('れ', "re"); m.insert('ろ', "ro");
        m.insert('わ', "wa"); m.insert('を', "wo"); m.insert('ん', "nn");

        // Voiced (Dakuten)
        m.insert('が', "ga"); m.insert('ぎ', "gi"); m.insert('ぐ', "gu"); m.insert('げ', "ge"); m.insert('ご', "go");
        m.insert('ざ', "za"); m.insert('じ', "ji"); m.insert('ず', "zu"); m.insert('ぜ', "ze"); m.insert('ぞ', "zo");
        m.insert('だ', "da"); m.insert('ぢ', "di"); m.insert('づ', "du"); m.insert('で', "de"); m.insert('ど', "do");
        m.insert('ば', "ba"); m.insert('び', "bi"); m.insert('ぶ', "bu"); m.insert('べ', "be"); m.insert('ぼ', "bo");

        // Semi-voiced (Handakuten)
        m.insert('ぱ', "pa"); m.insert('ぴ', "pi"); m.insert('ぷ', "pu"); m.insert('ぺ', "pe"); m.insert('ぽ', "po");

        // Small Kana
        m.insert('ぁ', "la"); m.insert('ぃ', "li"); m.insert('ぅ', "lu"); m.insert('ぇ', "le"); m.insert('ぉ', "lo");
        m.insert('っ', "ltu");
        m.insert('ゃ', "lya"); m.insert('ゅ', "lyu"); m.insert('ょ', "lyo");
        m.insert('ゎ', "lwa");

        m
    };
}

pub fn kana_to_romaji(c: char) -> Option<&'static str> {
    KANA_ROMAJI_MAP.get(&c).copied()
}

pub fn normalize_symbol(c: char) -> Option<char> {
    match c {
        '！' => Some('!'),
        '”' => Some('"'),
        '＃' => Some('#'),
        '＄' => Some('$'),
        '％' => Some('%'),
        '＆' => Some('&'),
        '’' => Some('\''),
        '（' => Some('('), // Override parser.rs fullwidth_shifted_keystroke logic if needed, or normalize first
        '）' => Some(')'),
        '＊' => Some('*'), // JIS :
        '＋' => Some('+'), // JIS ;
        '，' => Some(','),
        '－' => Some('-'),
        '．' => Some('.'),
        '／' => Some('/'),
        '：' => Some(':'),
        '；' => Some(';'),
        '＜' => Some('<'),
        '＝' => Some('='),
        '＞' => Some('>'),
        '？' => Some('?'),
        '＠' => Some('@'),
        '［' => Some('['),
        '＼' | '￥' => Some('\\'),
        '］' => Some(']'),
        '＾' => Some('^'),
        '＿' => Some('_'),
        '‘' => Some('\''),
        '｛' => Some('{'),
        '｜' => Some('|'),
        '｝' => Some('}'),
        '～' => Some('~'),
        '　' => Some(' '),

        // Punctuation that should be mapped to specific Scancodes in engine but normalized to ASCII here if possible?
        // Wait, '、' -> ',' for scancode purpose but we want to input '、' via IME.
        // Actually, if we type ',' in JP IME mode, we get '、'.
        '、' => Some(','),
        '。' => Some('.'),
        '・' => Some('/'),
        '「' => Some('['),
        '」' => Some(']'),

        _ => None,
    }
}

pub fn is_smart_symbol(c: char) -> bool {
    matches!(c, '“' | '”' | '‘' | '’')
}
