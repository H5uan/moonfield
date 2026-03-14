/*
 * Wrapper header for bindgen to process Slang C API
 *
 * This header provides the necessary type definitions and platform macros
 * for bindgen to properly parse the Slang C API.
 */

#ifndef SLANG_RS_WRAPPER_H
#define SLANG_RS_WRAPPER_H

/* === Platform Detection === */

#ifndef SLANG_COMPILER
    #define SLANG_COMPILER
    #ifdef _MSC_VER
        #define SLANG_VC 14
    #else
        #define SLANG_VC 0
    #endif
    #define SLANG_CLANG 0
    #define SLANG_SNC 0
    #define SLANG_GHS 0
    #define SLANG_GCC 0
#endif

#ifndef SLANG_PLATFORM
    #define SLANG_PLATFORM
    #if defined(_WIN64)
        #define SLANG_WIN64 1
        #define SLANG_WIN32 0
    #elif defined(_WIN32)
        #define SLANG_WIN64 0
        #define SLANG_WIN32 1
    #else
        #define SLANG_WIN64 0
        #define SLANG_WIN32 0
    #endif
    #define SLANG_WINRT 0
    #define SLANG_XBOXONE 0
    #define SLANG_X360 0
    #define SLANG_ANDROID 0
    #define SLANG_LINUX 0
    #define SLANG_IOS 0
    #define SLANG_OSX 0
    #define SLANG_PS3 0
    #define SLANG_PS4 0
    #define SLANG_PSP2 0
    #define SLANG_WIIU 0
    #define SLANG_WASM 0
#endif

/* Processor detection */
#ifndef SLANG_PROCESSOR_X86_64
    #if defined(_M_AMD64) || defined(_M_X64) || defined(__amd64) || defined(__x86_64)
        #define SLANG_PROCESSOR_X86_64 1
        #define SLANG_PROCESSOR_X86 0
        #define SLANG_PROCESSOR_ARM 0
        #define SLANG_PROCESSOR_ARM_64 0
    #elif defined(__i386__) || defined(_M_IX86)
        #define SLANG_PROCESSOR_X86_64 0
        #define SLANG_PROCESSOR_X86 1
        #define SLANG_PROCESSOR_ARM 0
        #define SLANG_PROCESSOR_ARM_64 0
    #else
        #define SLANG_PROCESSOR_X86_64 0
        #define SLANG_PROCESSOR_X86 0
        #define SLANG_PROCESSOR_ARM 0
        #define SLANG_PROCESSOR_ARM_64 0
    #endif
#endif

#ifndef SLANG_PROCESSOR_POWER_PC
    #define SLANG_PROCESSOR_POWER_PC 0
    #define SLANG_PROCESSOR_POWER_PC_64 0
#endif

#ifndef SLANG_PROCESSOR_WASM
    #define SLANG_PROCESSOR_WASM 0
#endif

/* Derived macros */
#define SLANG_GCC_FAMILY (SLANG_CLANG || SLANG_SNC || SLANG_GHS || SLANG_GCC)
#define SLANG_WINDOWS_FAMILY (SLANG_WINRT || SLANG_WIN32 || SLANG_WIN64)
#define SLANG_MICROSOFT_FAMILY (SLANG_XBOXONE || SLANG_X360 || SLANG_WINDOWS_FAMILY)
#define SLANG_LINUX_FAMILY (SLANG_LINUX || SLANG_ANDROID)
#define SLANG_APPLE_FAMILY (SLANG_IOS || SLANG_OSX)
#define SLANG_UNIX_FAMILY (SLANG_LINUX_FAMILY || SLANG_APPLE_FAMILY)

#define SLANG_PROCESSOR_FAMILY_X86 (SLANG_PROCESSOR_X86_64 | SLANG_PROCESSOR_X86)
#define SLANG_PROCESSOR_FAMILY_ARM (SLANG_PROCESSOR_ARM | SLANG_PROCESSOR_ARM_64)
#define SLANG_PROCESSOR_FAMILY_POWER_PC (SLANG_PROCESSOR_POWER_PC_64 | SLANG_PROCESSOR_POWER_PC)

#define SLANG_PTR_IS_64 (SLANG_PROCESSOR_ARM_64 | SLANG_PROCESSOR_X86_64 | SLANG_PROCESSOR_POWER_PC_64)
#define SLANG_PTR_IS_32 (SLANG_PTR_IS_64 ^ 1)

#define SLANG_LITTLE_ENDIAN 1
#define SLANG_BIG_ENDIAN 0
#define SLANG_UNALIGNED_ACCESS SLANG_PROCESSOR_FAMILY_X86

/* DirectX configuration */
#define SLANG_ENABLE_DXVK 0
#define SLANG_ENABLE_VKD3D 0
#define SLANG_ENABLE_DIRECTX SLANG_WINDOWS_FAMILY
#define SLANG_ENABLE_DXGI_DEBUG SLANG_WINDOWS_FAMILY
#define SLANG_ENABLE_DXBC_SUPPORT SLANG_WINDOWS_FAMILY
#define SLANG_ENABLE_PIX SLANG_WINDOWS_FAMILY

/* Exception handling */
#define SLANG_HAS_EXCEPTIONS 0
#define SLANG_NO_THROW

/* Calling conventions */
#ifndef SLANG_STDCALL
    #if SLANG_MICROSOFT_FAMILY
        #define SLANG_STDCALL __stdcall
    #else
        #define SLANG_STDCALL
    #endif
#endif
#ifndef SLANG_MCALL
    #define SLANG_MCALL SLANG_STDCALL
#endif

/* Linkage */
#define SLANG_DLL_EXPORT
#define SLANG_API

/* Utility macros */
#define SLANG_NO_INLINE
#define SLANG_FORCE_INLINE inline
#define SLANG_INLINE inline
#define SLANG_OVERRIDE
#define SLANG_UNUSED(v) (void)v;
#define SLANG_MAYBE_UNUSED
#define SLANG_COMPILE_TIME_ASSERT(x) static_assert(x)
#define SLANG_BREAKPOINT(id) (*((int*)0) = int(id));
#define SLANG_OFFSET_OF(T, ELEMENT) offsetof(T, ELEMENT)
#define SLANG_COUNT_OF(x) (SlangSSizeT(sizeof(x) / sizeof(0 [x])))
#define SLANG_STRINGIZE_HELPER(X) #X
#define SLANG_STRINGIZE(X) SLANG_STRINGIZE_HELPER(X)
#define SLANG_CONCAT_HELPER(X, Y) X##Y
#define SLANG_CONCAT(X, Y) SLANG_CONCAT_HELPER(X, Y)

#define SLANG_INT64(x) (x##ll)
#define SLANG_UINT64(x) (x##ull)

#define SLANG_DEPRECATED

/* Backtrace */
#define SLANG_HAS_BACKTRACE 0

/* Standard includes needed for bindgen */
#include <inttypes.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdint.h>

/* Include the main Slang C API header */
#include "include/slang.h"

#endif /* SLANG_RS_WRAPPER_H */
