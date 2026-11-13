// Stores to forced variables are blocked (also reals); real arrays read 0.0 out of range
module top;
  reg d;
  real f = 0.0, z = 0.0;
  reg [7:0] g = 0, zz = 0;
  reg [7:0] h, hz;
  real ra [0:3];
  integer i;
  always @(d) begin force f = z; force g = zz; end
  initial begin
    hz = 0;
    force h = hz;
    for (i = 0; i < 4; i = i + 1) ra[i] = i + 0.5;
    #1 d = 0;
    #1 f = 1.0; g = 5; h = 5;
    $display("%f %0d %0d", f, g, h);
    i = 'bx;
    $display("%f %f %f", ra[i], ra[7], ra[2]);
    release h;
    h = 7;
    $display("%0d", h);
  end
endmodule
