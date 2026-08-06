cmake_minimum_required(VERSION 3.20)
project(wellfriend_pdfium_harness C)

set(PDFIUM_ROOT "" CACHE PATH "Official PDFium SDK root containing public/fpdfview.h and libpdfium")

find_path(
  PDFIUM_INCLUDE_DIR
  NAMES fpdfview.h
  HINTS "${PDFIUM_ROOT}"
  PATH_SUFFIXES public include
  REQUIRED
)
find_library(
  PDFIUM_LIBRARY
  NAMES pdfium libpdfium
  HINTS "${PDFIUM_ROOT}"
  PATH_SUFFIXES lib lib64 bin
  REQUIRED
)

add_executable(wellfriend-pdfium-harness render_page.c)
target_compile_features(wellfriend-pdfium-harness PRIVATE c_std_11)
target_include_directories(wellfriend-pdfium-harness PRIVATE "${PDFIUM_INCLUDE_DIR}")
target_link_libraries(wellfriend-pdfium-harness PRIVATE "${PDFIUM_LIBRARY}")

if(MSVC)
  target_compile_options(wellfriend-pdfium-harness PRIVATE /W4 /WX)
else()
  target_compile_options(wellfriend-pdfium-harness PRIVATE -Wall -Wextra -Werror -pedantic)
  target_link_libraries(wellfriend-pdfium-harness PRIVATE m)
endif()

install(TARGETS wellfriend-pdfium-harness RUNTIME DESTINATION bin)
