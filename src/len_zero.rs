extern crate rustc_typeck as typeck;

use std::rc::Rc;
use std::cell::RefCell;
use syntax::ptr::P;
use rustc::lint::{Context, LintPass, LintArray, Lint};
use rustc::util::nodemap::DefIdMap;
use rustc::middle::ty::{self, node_id_to_type, sty, ty_ptr, ty_rptr, expr_ty,
	mt, ty_to_def_id, impl_or_trait_item, MethodTraitItemId, ImplOrTraitItemId};
use rustc::middle::def::{DefTy, DefStruct, DefTrait};
use syntax::codemap::{Span, Spanned};
use syntax::ast::*;
use misc::walk_ty;

declare_lint!(pub LEN_ZERO, Warn,
              "Warn when .is_empty() could be used instead of checking .len()");

declare_lint!(pub LEN_WITHOUT_IS_EMPTY, Warn,
              "Warn on traits and impls that have .len() but not .is_empty()");

#[derive(Copy,Clone)]
pub struct LenZero;

impl LintPass for LenZero {
	fn get_lints(&self) -> LintArray {
        lint_array!(LEN_ZERO, LEN_WITHOUT_IS_EMPTY)
	}
	
	fn check_item(&mut self, cx: &Context, item: &Item) {
		match &item.node {
			&ItemTrait(_, _, _, ref trait_items) => 
				check_trait_items(cx, item, trait_items),
			&ItemImpl(_, _, _, None, _, ref impl_items) => // only non-trait
				check_impl_items(cx, item, impl_items),
			_ => ()
		}
	}
	
	fn check_expr(&mut self, cx: &Context, expr: &Expr) {
		if let &ExprBinary(Spanned{node: cmp, ..}, ref left, ref right) = 
				&expr.node {
			match cmp {
				BiEq => check_cmp(cx, expr.span, left, right, ""),
				BiGt | BiNe => check_cmp(cx, expr.span, left, right, "!"),
				_ => ()
			}
		}
	}
}

fn check_trait_items(cx: &Context, item: &Item, trait_items: &[P<TraitItem>]) {
	fn is_named_self(item: &TraitItem, name: &str) -> bool {
		item.ident.as_str() == name && if let MethodTraitItem(ref sig, _) =
			item.node { is_self_sig(sig) } else { false }
	}

	if !trait_items.iter().any(|i| is_named_self(i, "is_empty")) {
		//cx.span_lint(LEN_WITHOUT_IS_EMPTY, item.span, &format!("trait {}", item.ident.as_str()));
		for i in trait_items {
			if is_named_self(i, "len") {
				cx.span_lint(LEN_WITHOUT_IS_EMPTY, i.span,
					&format!("Trait '{}' has a '.len(_: &Self)' method, but no \
						'.is_empty(_: &Self)' method. Consider adding one.", 
						item.ident.as_str()));
			}
		};
	}
}

fn check_impl_items(cx: &Context, item: &Item, impl_items: &[P<ImplItem>]) {
	fn is_named_self(item: &ImplItem, name: &str) -> bool {
		item.ident.as_str() == name && if let MethodImplItem(ref sig, _) = 
				item.node { is_self_sig(sig) } else { false }
	}

	if !impl_items.iter().any(|i| is_named_self(i, "is_empty")) {
		for i in impl_items {
			if is_named_self(i, "len") {
				let s = i.span;
				cx.span_lint(LEN_WITHOUT_IS_EMPTY, 
					Span{ lo: s.lo, hi: s.lo, expn_id: s.expn_id },
					&format!("Item '{}' has a '.len(_: &Self)' method, but no \
						'.is_empty(_: &Self)' method. Consider adding one.", 
						item.ident.as_str()));
				return;
			}
		}
	}
}

fn is_self_sig(sig: &MethodSig) -> bool {
	if let SelfStatic = sig.explicit_self.node { 
		false } else { sig.decl.inputs.len() == 1 }
}

fn check_cmp(cx: &Context, span: Span, left: &Expr, right: &Expr, empty: &str) {
	match (&left.node, &right.node) {
		(&ExprLit(ref lit), &ExprMethodCall(ref method, _, ref args)) => 
			check_len_zero(cx, span, method, args, lit, empty),
		(&ExprMethodCall(ref method, _, ref args), &ExprLit(ref lit)) => 
			check_len_zero(cx, span, method, args, lit, empty),
		_ => ()
	}
}

fn check_len_zero(cx: &Context, span: Span, method: &SpannedIdent, 
		args: &[P<Expr>], lit: &Lit, empty: &str) {
	if let &Spanned{node: LitInt(0, _), ..} = lit {
		if method.node.as_str() == "len" && args.len() == 1 &&
			has_is_empty(cx, &*args[0]) {
			cx.span_lint(LEN_ZERO, span, &format!(
				"Consider replacing the len comparison with '{}_.is_empty()'",
					empty))
		}
	}
}

/// check if this type has an is_empty method
fn has_is_empty(cx: &Context, expr: &Expr) -> bool {
	/// get a ImplOrTraitItem and return true if it matches is_empty(self)
	fn is_is_empty(cx: &Context, id: &ImplOrTraitItemId) -> bool {
		if let &MethodTraitItemId(def_id) = id {
			if let ty::MethodTraitItem(ref method) = 
					ty::impl_or_trait_item(cx.tcx, def_id) {
				method.name.as_str() == "is_empty"
					&& method.fty.sig.skip_binder().inputs.len() == 1 
			} else { false }
		} else { false }
	}
	
	/// check the inherent impl's items for an is_empty(self) method
	fn has_is_empty_impl(cx: &Context, id: &DefId) -> bool {
		let impl_items = cx.tcx.impl_items.borrow();
		cx.tcx.inherent_impls.borrow().get(id).map_or(false, 
			|ids| ids.iter().any(|iid| impl_items.get(iid).map_or(false, 
				|iids| iids.iter().any(|i| is_is_empty(cx, i)))))
	}
	
	let ty = &walk_ty(&expr_ty(cx.tcx, expr));
	match ty.sty {
		ty::ty_trait(_) => cx.tcx.trait_item_def_ids.borrow().get(
			&ty::ty_to_def_id(ty).expect("trait impl not found")).map_or(false, 
			|ids| ids.iter().any(|i| is_is_empty(cx, i))),
		ty::ty_projection(_) => ty::ty_to_def_id(ty).map_or(false, 
			|id| has_is_empty_impl(cx, &id)),
		ty::ty_enum(ref id, _) | ty::ty_struct(ref id, _) => 
			has_is_empty_impl(cx, id),
		ty::ty_vec(..) => true,
		_ => false,
	}
}
