When performing a code review, check that:
- No unit tests were eliminated without a strong reason.
- Additional dependencies introduced have a strong justification.
- Changes are not likely to increase build times.
- Each file has a license header appropriate to its provenance. Preserve upstream and third-party copyright notices, including in modified files. The existing upstream header is:
```
/*
 * Copyright (c) Microsoft Corporation.
 * Licensed under the MIT license.
 */
```

For project-authored code, use the following style, retaining recorded years:
```
// Copyright (C) 2026 TerseMind.
// Use of this source code is governed by a MIT license.
```
In mixed-source files, append a TerseMind notice scoped to the RSI modifications; keep the original upstream header. Do not copy the Microsoft notice into new project-authored files.
