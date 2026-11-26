// `edge` event controls (IEEE 1800-2017 9.4.2): a posedge or a negedge, also those to and
// from x and z; x to z is neither
module top;
  logic s = 0;
  int n = 0;
  always @(edge s) n++;
  initial begin
    #1 s = 1;    // posedge
    #1 s = 0;    // negedge
    #1 s = 1'bx; // posedge
    #1 s = 0;    // negedge
    #1 s = 1'bz; // posedge
    #1 s = 1;    // posedge
    #1 s = 1'bx; // negedge
    #1 s = 1'bz; // none
    #1 if (n == 7) $display("PASSED");
    else $display("FAILED %0d", n);
  end
endmodule
