// Releasing a forced net gives it its drivers' value again, also when the driver is a
// constant (which runs only once) or a word of a net array.
module top;
  wire [1:0] w;
  wire [1:0] arr [1:2];
  wire [3:0] p;
  reg [1:0] r;
  assign w = 2'd0;
  assign arr[1] = 2'd0;
  assign arr[2] = r;
  assign p[1:0] = 2'b10;
  assign p[3:2] = r;
  initial begin
    r = 2;
    force w = 1; force arr[1] = 1; force arr[2] = 3; force p = 4'hf;
    #1 $display("%h %h %h %h", w, arr[1], arr[2], p);
    release w; release arr[1]; release arr[2]; release p;
    #1 $display("%h %h %h %h", w, arr[1], arr[2], p);
    force p[1:0] = 2'b01;
    #1 $display("%b", p);
    release p[1:0];
    #1 $display("%b", p);
  end
endmodule
