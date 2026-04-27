// Reproduces the layerchart "bare Component<{}>" assignment pattern —
// the case the `& { $set?: any; $on?: any }` intersection was added
// to fix in commit fd126e98.
//
// Tests three source shapes (ours, upstream-iso, Component<>) against
// the bare `Component<>` target.

import type { Component } from 'svelte';
import { SvgOurs, SvgUp, SvgComponent } from './svg_component.ts';

// EXPECT CLEAN — ours has `& { $set?, $on? }` so callable return matches.
const targetOurs1: Component = SvgOurs;
const targetOurs2: Component<{}> = SvgOurs;

// EXPECT CLEAN or FAIL — upstream's per-component iso lacks `& { $set?, $on? }`
// — this is the path we'd switch to to fix Threlte.
const targetUp1: Component = SvgUp;
const targetUp2: Component<{}> = SvgUp;

// EXPECT CLEAN — Component<> trivially is itself.
const targetComp1: Component = SvgComponent;
const targetComp2: Component<{}> = SvgComponent;

void targetOurs1;
void targetOurs2;
void targetUp1;
void targetUp2;
void targetComp1;
void targetComp2;
