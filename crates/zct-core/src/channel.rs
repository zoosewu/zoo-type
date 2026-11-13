//! 正式版與開發版：兩套完全獨立的輸入法，可以同時安裝，開發版壞了不影響每天在用的正式版
//!
//! 建置時用 cargo feature `zct-core/dev` 選擇（`cargo build-dev`）。所有「這是哪一套」的識別都在這裡：
//! TSF 的 CLSID 與語言設定檔、顯示名稱、資料與安裝資料夾、連線管道、mutex、認證管理員的名稱。

/// 這一套的識別
pub struct Channel {
    /// `"official"` / `"dev"`（安裝腳本用來確認拿到的是哪一套）
    pub id: &'static str,
    /// 鍵盤清單、輸入法圖示、視窗標題顯示的名稱
    pub name: &'static str,
    /// 資料資料夾（`%APPDATA%\<dir>`）、安裝資料夾（`Program Files\<dir>`）與各種系統名稱的前綴
    pub dir: &'static str,
    /// 連線管道名稱的前綴（`\\.\pipe\<pipe>-v<協定>-<SID>`）
    pub pipe: &'static str,
    /// TSF 文字服務的 CLSID
    pub clsid: u128,
    /// 語言設定檔
    pub profile: u128,
    /// 組字與選字目標的顯示屬性
    pub attr_input: u128,
    pub attr_target: u128,
    /// 右 Alt 保留按鍵 GUID 的基底（低 16 位元放按鍵與 Shift）
    pub preserved_keys: u128,
}

pub const OFFICIAL: Channel = Channel {
    id: "official",
    name: "動物園輸入法",
    dir: "ZooType",
    pipe: "zct",
    clsid: 0x68e048cc_2635_4a97_8a2f_1599af3afcf5,
    profile: 0xd890a24e_551e_4d5f_a0f1_745be13b6287,
    attr_input: 0x5b2c6a9e_7c1d_4f3e_9a51_2e8f0d4c7b61,
    attr_target: 0x0f8e3d2c_1a4b_4c6d_8e9f_a1b2c3d4e5f6,
    preserved_keys: 0x3c0e5a71_9d42_4b6e_8f13_a2b4c6d80000,
};

pub const DEV: Channel = Channel {
    id: "dev",
    name: "動物園輸入法 開發版",
    dir: "ZooType-dev",
    pipe: "zct-dev",
    clsid: 0xa45461cc_ec30_4b88_bada_0f20a6fd0c19,
    profile: 0xb28977f5_a5ac_4c38_8817_fc51e1f79919,
    attr_input: 0x42653309_d226_4a72_80fd_b7c9af3eee17,
    attr_target: 0xfeede65d_d8dd_4f18_a9ef_739f2893d85c,
    preserved_keys: 0xb1f1c6a5_35d3_4b5a_8c54_a26dffa50000,
};

/// 這次建置是哪一套
#[cfg(not(feature = "dev"))]
pub const CURRENT: Channel = OFFICIAL;
#[cfg(feature = "dev")]
pub const CURRENT: Channel = DEV;

/// 設定程式的視窗標題（也用來找已開著的視窗）
pub fn settings_title() -> String {
    format!("{} 設定", CURRENT.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_share_nothing() {
        let ids = |c: &Channel| {
            [
                c.clsid,
                c.profile,
                c.attr_input,
                c.attr_target,
                c.preserved_keys,
            ]
        };
        for a in ids(&OFFICIAL) {
            assert!(!ids(&DEV).contains(&a));
        }
        assert_ne!(OFFICIAL.dir, DEV.dir);
        assert_ne!(OFFICIAL.pipe, DEV.pipe);
        assert_ne!(OFFICIAL.name, DEV.name);
        assert_eq!(DEV.preserved_keys & 0xffff, 0, "低 16 位元留給按鍵");
    }
}
