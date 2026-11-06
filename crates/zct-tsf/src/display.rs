//! 組字區顯示屬性（使用者回饋：組字區要更明顯，並標出按 ↓ 會影響的字）
//!
//! - 整段組字：虛線底線
//! - 選字目標：粗實線底線 + 淺色底
//!
//! 由 App 依顯示屬性自己畫，所以不支援的 App（部分終端機）會沒有效果；服務程序另有組字提示列。

use std::cell::Cell;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::COLORREF;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_I4};
use windows::Win32::UI::TextServices::*;
use windows::core::{BSTR, GUID, Result, implement};

use crate::guard;

pub const ATTR_INPUT: GUID = GUID::from_u128(0x5b2c6a9e_7c1d_4f3e_9a51_2e8f0d4c7b61);
pub const ATTR_TARGET: GUID = GUID::from_u128(0x0f8e3d2c_1a4b_4c6d_8e9f_a1b2c3d4e5f6);

fn color(cr: u32) -> TF_DA_COLOR {
    TF_DA_COLOR {
        r#type: TF_CT_COLORREF,
        Anonymous: TF_DA_COLOR_0 { cr: COLORREF(cr) },
    }
}

fn attribute(guid: &GUID) -> TF_DISPLAYATTRIBUTE {
    if *guid == ATTR_TARGET {
        TF_DISPLAYATTRIBUTE {
            crBk: color(0x00F7E4CC), // #CCE4F7（GDI 是 BGR），與候選窗選擇框同色
            lsStyle: TF_LS_SOLID,
            fBoldLine: true.into(),
            bAttr: TF_ATTR_TARGET_CONVERTED,
            ..Default::default()
        }
    } else {
        TF_DISPLAYATTRIBUTE {
            lsStyle: TF_LS_DOT,
            bAttr: TF_ATTR_INPUT,
            ..Default::default()
        }
    }
}

#[implement(ITfDisplayAttributeInfo)]
pub struct AttributeInfo {
    guid: GUID,
}

impl ITfDisplayAttributeInfo_Impl for AttributeInfo_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(self.guid)
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from(if self.guid == ATTR_TARGET {
            "ZCT 選字目標"
        } else {
            "ZCT 組字"
        }))
    }

    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        guard(|| {
            if pda.is_null() {
                return Err(windows::Win32::Foundation::E_INVALIDARG.into());
            }
            unsafe { *pda = attribute(&self.guid) };
            Ok(())
        })
    }

    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

pub fn info(guid: &GUID) -> Option<ITfDisplayAttributeInfo> {
    (*guid == ATTR_INPUT || *guid == ATTR_TARGET).then(|| AttributeInfo { guid: *guid }.into())
}

#[implement(IEnumTfDisplayAttributeInfo)]
pub struct AttributeEnum {
    pos: Cell<usize>,
}

impl AttributeEnum {
    pub fn create() -> IEnumTfDisplayAttributeInfo {
        AttributeEnum { pos: Cell::new(0) }.into()
    }
}

const ALL: [GUID; 2] = [ATTR_INPUT, ATTR_TARGET];

impl IEnumTfDisplayAttributeInfo_Impl for AttributeEnum_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(AttributeEnum {
            pos: Cell::new(self.pos.get()),
        }
        .into())
    }

    fn Next(&self, count: u32, out: *mut Option<ITfDisplayAttributeInfo>, fetched: *mut u32) -> Result<()> {
        guard(|| unsafe {
            let mut n = 0u32;
            while n < count && self.pos.get() < ALL.len() {
                *out.add(n as usize) = info(&ALL[self.pos.get()]);
                self.pos.set(self.pos.get() + 1);
                n += 1;
            }
            if !fetched.is_null() {
                *fetched = n;
            }
            if n == count {
                Ok(())
            } else {
                Err(windows::Win32::Foundation::S_FALSE.into())
            }
        })
    }

    fn Reset(&self) -> Result<()> {
        self.pos.set(0);
        Ok(())
    }

    fn Skip(&self, count: u32) -> Result<()> {
        self.pos.set((self.pos.get() + count as usize).min(ALL.len()));
        Ok(())
    }
}

/// 兩種顯示屬性的 GUID atom（ActivateEx 時註冊）
#[derive(Clone, Copy, Default)]
pub struct Atoms {
    input: u32,
    target: u32,
}

impl Atoms {
    pub fn register() -> Option<Atoms> {
        unsafe {
            let cats: ITfCategoryMgr =
                CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER).ok()?;
            Some(Atoms {
                input: cats.RegisterGUID(&ATTR_INPUT).ok()?,
                target: cats.RegisterGUID(&ATTR_TARGET).ok()?,
            })
        }
    }
}

fn atom_variant(atom: u32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_I4,
                Anonymous: VARIANT_0_0_0 { lVal: atom as i32 },
                ..Default::default()
            }),
        },
    }
}

/// 把顯示屬性套到組字範圍；`target` 是（UTF-16 起點, UTF-16 長度）
pub fn apply(ctx: &ITfContext, ec: u32, range: &ITfRange, atoms: Atoms, target: Option<(i32, i32)>) {
    unsafe {
        let Ok(prop) = ctx.GetProperty(&GUID_PROP_ATTRIBUTE) else {
            return;
        };
        let _ = prop.SetValue(ec, range, &atom_variant(atoms.input));
        let Some((start, len)) = target.filter(|(_, len)| *len > 0) else {
            return;
        };
        let Ok(sub) = range.Clone() else { return };
        let mut moved = 0;
        if sub.Collapse(ec, TF_ANCHOR_START).is_err()
            || sub.ShiftEnd(ec, start + len, &mut moved, std::ptr::null()).is_err()
            || sub.ShiftStart(ec, start, &mut moved, std::ptr::null()).is_err()
        {
            return;
        }
        let _ = prop.SetValue(ec, &sub, &atom_variant(atoms.target));
    }
}
