/* WinSparkle-NG version information.
 * Copyright (c) 2026 Gianluigi Tiesi <sherpya@gmail.com>
 * SPDX-License-Identifier: MIT
 */
#ifndef WINSPARKLE_VERSION_H
#define WINSPARKLE_VERSION_H

#define WIN_SPARKLE_VERSION_MAJOR 1
#define WIN_SPARKLE_VERSION_MINOR 0
#define WIN_SPARKLE_VERSION_MICRO 0
#define WIN_SPARKLE_VERSION_STRING "1.0.0"
#define WIN_SPARKLE_CHECK_VERSION(major, minor, micro) \
    ((WIN_SPARKLE_VERSION_MAJOR > (major)) || \
     (WIN_SPARKLE_VERSION_MAJOR == (major) && WIN_SPARKLE_VERSION_MINOR > (minor)) || \
     (WIN_SPARKLE_VERSION_MAJOR == (major) && WIN_SPARKLE_VERSION_MINOR == (minor) && \
      WIN_SPARKLE_VERSION_MICRO >= (micro)))

#endif
