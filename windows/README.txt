FACET simulator for Windows (test build)
========================================

Requirements: 64-bit Windows 10 or 11, and a graphics card with current
drivers (OpenGL 3). Virtual machines and Remote Desktop sessions often have
no OpenGL 3; the program then shows an error. There, use the download
facet-sim-windows-software-gl.zip instead (same program, with a software
OpenGL renderer; slower, but it runs anywhere).

Running it
----------
1. Double-click facet-sim.exe.
2. The build is not code-signed, so Windows shows "Windows protected your
   PC". Click "More info", then "Run anyway". (Only the first time.)
3. The simulator window opens with 1,000 telescopes in orbit.

Options are given on the command line, e.g. from PowerShell in this folder:
    .\facet-sim.exe --n 10000 --rate 600
Output from the program appears in that terminal.

Please check, and note anything that fails or looks wrong
---------------------------------------------------------
[ ] The window opens; the icon (blue hexagon "compound eye") is on the
    window and in the taskbar.
[ ] The globe turns smoothly; dragging rotates it, the mouse wheel zooms.
[ ] The speed control works (try 60x and 600x); it stays smooth.
[ ] Open a browser at http://localhost:8781/ (public requests page) and
    http://localhost:8780/ (control page). Both load. Windows Firewall
    should NOT ask for permission (the pages are local only).
[ ] Submit a request on the public page; it appears and gets observed.
[ ] Presentations: start one; the narration plays through the speakers.
[ ] Save a scenario, close the program, start it again and load it.
[ ] Close the program with the window's X; it exits cleanly.
[ ] Right-click facet-sim.exe > Properties > Details shows
    "FACET simulator" and the version.

If something fails, a screenshot plus the exact Windows version
(Settings > System > About) and graphics card helps most.
