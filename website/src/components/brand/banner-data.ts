// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.components.brand.banner-data
// Responsibility:   The drawings of the start banner of the program nurt as shipped in release 0.1.1 (half blocks and 7-bit ASCII) with the role of each cell; the source is crates/nurt-contracts/src/product.rs.
// Dependencies:     none.

export const bannerBlocks = [
  '           ▄██',
  '        ▄▄████',
  '      ▄██████▀',
  '   ▄██████▀',
  ' ▄█████▀▀   ▄▄▄',
  '██████▄     ████▄',
  ' ▀██████▄   ▀██████▄',
  '   ▀▀██████▄  ▀██████▄▄',
  '      ▀██████▄   ▀██████▄',
  '         ▀████     ▀██████',
  '           ▀▀▀   ▄▄█████▀',
  '               ▄██████▀',
  '            ▄██████▀',
  '            ████▀▀',
  '            ██▀',
] as const;

export const bannerBlocksRoles = [
  '           sss',
  '        ssssss',
  '      ssssssss',
  '   ssssssss',
  ' ssssssss   ggg',
  'sssssss     ggggg',
  ' ssssssss   gggggggg',
  '   sssssssss  ggggggggg',
  '      ssssssss   gggggggg',
  '         sssss     ggggggg',
  '           sss   gggggggg',
  '               gggggggg',
  '            gggggggg',
  '            gggggg',
  '            ggg',
] as const;

export const bannerAscii = [
  '          +#+',
  '       :+###+',
  '    :+######:',
  '  +######+',
  '+#####+:   +#+:',
  '######+    +####+',
  ' :+######: :+#####+:',
  '    :+#####+: :######+:',
  '       +####+    +######',
  '         :+#+   :+#####+',
  '              +######+',
  '           :######+:',
  '           +###+:',
  '           +#+',
] as const;

export const bannerAsciiRoles = [
  '          sss',
  '       ssssss',
  '    sssssssss',
  '  ssssssss',
  'ssssssss   gggg',
  'sssssss    gggggg',
  ' sssssssss ggggggggg',
  '    sssssssss ggggggggg',
  '       ssssss    ggggggg',
  '         ssss   gggggggg',
  '              gggggggg',
  '           ggggggggg',
  '           gggggg',
  '           ggg',
] as const;

export const bannerGap = 3;
export const bannerTextRows = [3, 5, 7, 9, 11] as const;
export const bannerHint = "Type 'help' to get started.";
