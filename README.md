doxygen2man:

This is a tool to generate API manpages from a doxygen-annotated header file.
First run doxygen on the file and then run this program against the main XML file
it created and the directory containing the ancilliary files. It will then
output a lot of *.3 man page files which you can then ship with your library.
You will need to invoke this program on each .h file in your library (one 
invocation can contain multiple files),
using the name of the generated .xml file. This file will usually be called
something like <include-file>_8h.xml, eg qbipcs_8h.xml
If you want HTML output then simpy use nroff on the generated files as you
would do with any other man page.
