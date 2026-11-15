`define WIDTH 6
`timescale 1ns/1ns
typedef logic [`WIDTH-1:0] word_t;
int counter;
package q;
  localparam int N = 3;
  typedef struct packed { word_t w; logic f; } rec_t;
endpackage
function automatic int inc(int x); return x + 1; endfunction
