cmake_minimum_required(VERSION 3.24)
project(@PACKAGE@ VERSION @VERSION@ LANGUAGES CXX)
include(GNUInstallDirs)
include(CMakePackageConfigHelpers)

option(SUSPECT_SDK_WITH_CURL "Build the recommended libcurl adapter" ON)
option(SUSPECT_SDK_BUILD_EXAMPLES "Build and test executable source examples" ${PROJECT_IS_TOP_LEVEL})
option(SUSPECT_SDK_BUILD_DOCS "Build Doxygen native symbol documentation" OFF)
option(SUSPECT_SDK_WARNINGS_AS_ERRORS "Treat SDK compilation warnings as errors" ON)

add_library(@PACKAGE@ src/runtime.cpp src/validation.cpp src/validation_v2.cpp src/validation_v3.cpp src/program.cpp src/models.cpp src/http.cpp src/wire.cpp src/payload.cpp src/stream.cpp src/client.cpp)
add_library(@PACKAGE@::@PACKAGE@ ALIAS @PACKAGE@)
target_compile_features(@PACKAGE@ PUBLIC cxx_std_20)
set_target_properties(@PACKAGE@ PROPERTIES CXX_EXTENSIONS OFF POSITION_INDEPENDENT_CODE ON VERSION ${PROJECT_VERSION} SOVERSION ${PROJECT_VERSION_MAJOR})
target_include_directories(@PACKAGE@ PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:${CMAKE_INSTALL_INCLUDEDIR}>)
find_package(Threads REQUIRED)
target_link_libraries(@PACKAGE@ PUBLIC Threads::Threads)
if(SUSPECT_SDK_WITH_CURL)
    find_package(CURL 7.85 REQUIRED)
    target_sources(@PACKAGE@ PRIVATE src/curl_transport.cpp)
    target_compile_definitions(@PACKAGE@ PUBLIC @PACKAGE@_HAS_CURL=1)
    target_link_libraries(@PACKAGE@ PUBLIC CURL::libcurl)
endif()

function(sdk_warnings target)
    if(MSVC)
        target_compile_options(${target} PRIVATE /W4)
        if(SUSPECT_SDK_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE /WX)
        endif()
    elseif(CMAKE_CXX_COMPILER_ID MATCHES "Clang|GNU")
        target_compile_options(${target} PRIVATE -Wall -Wextra -Wpedantic)
        if(SUSPECT_SDK_WARNINGS_AS_ERRORS)
            target_compile_options(${target} PRIVATE -Werror)
        endif()
    endif()
endfunction()
sdk_warnings(@PACKAGE@)

if(SUSPECT_SDK_BUILD_EXAMPLES)
    enable_testing()
    foreach(example IN ITEMS validated_examples client)
        if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/examples/${example}.cpp")
            add_executable(sdk_${example} examples/${example}.cpp)
            target_link_libraries(sdk_${example} PRIVATE @PACKAGE@::@PACKAGE@)
            sdk_warnings(sdk_${example})
            add_test(NAME sdk_${example} COMMAND sdk_${example})
        endif()
    endforeach()
endif()

if(SUSPECT_SDK_BUILD_DOCS)
    find_package(Doxygen 1.9.8 REQUIRED)
    configure_file(Doxyfile "${CMAKE_CURRENT_BINARY_DIR}/Doxyfile" @ONLY)
    add_custom_target(sdk_docs
        COMMAND Doxygen::doxygen "${CMAKE_CURRENT_BINARY_DIR}/Doxyfile"
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
        COMMENT "Building native C++ symbol reference" VERBATIM)
    install(DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/docs/html/" DESTINATION "${CMAKE_INSTALL_DOCDIR}/html" OPTIONAL)
endif()

install(TARGETS @PACKAGE@ EXPORT @PACKAGE@Targets
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(DIRECTORY include/ DESTINATION ${CMAKE_INSTALL_INCLUDEDIR})
install(EXPORT @PACKAGE@Targets NAMESPACE @PACKAGE@:: DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/@PACKAGE@)
configure_package_config_file(cmake/@PACKAGE@Config.cmake.in
    "${CMAKE_CURRENT_BINARY_DIR}/@PACKAGE@Config.cmake"
    INSTALL_DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/@PACKAGE@)
write_basic_package_version_file("${CMAKE_CURRENT_BINARY_DIR}/@PACKAGE@ConfigVersion.cmake"
    VERSION ${PROJECT_VERSION} COMPATIBILITY SameMajorVersion)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/@PACKAGE@Config.cmake" "${CMAKE_CURRENT_BINARY_DIR}/@PACKAGE@ConfigVersion.cmake"
    DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/@PACKAGE@)
install(FILES README.md sdk-manifest.json DESTINATION ${CMAKE_INSTALL_DOCDIR})
install(DIRECTORY docs DESTINATION ${CMAKE_INSTALL_DOCDIR})
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/examples")
    install(DIRECTORY examples DESTINATION ${CMAKE_INSTALL_DOCDIR})
endif()
