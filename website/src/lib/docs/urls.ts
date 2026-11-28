// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.urls
// Responsibility:   Logical path of a documentation page from the identifier of its entry.
// Dependencies:     website.config.docs

import { docsPathPrefix } from '../../../config/docs';

export function docsUrl(id: string): string {
  return id.endsWith('/index') ? `${docsPathPrefix}/${id.slice(0, -'/index'.length)}/` : `${docsPathPrefix}/${id}/`;
}
