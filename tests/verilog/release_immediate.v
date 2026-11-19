// Releasing (part of) a forced net gives the released bits their drivers' value at once,
// visible to the next statement, also when the driver changed while they were forced.
module test;
  reg [3:0] in = 4'b0;
  wire [3:0] bus = in;
  initial begin
    #1 force bus[0] = 1'b1;
    #1 in[0] = 1'bz;
    $display("1: %b", bus);
    #1 force bus[3:2] = 2'b11;
    $display("2: %b", bus);
    #1 release bus[0];
    $display("3: %b", bus);
    #1 release bus[3:2];
    $display("4: %b", bus);
  end
endmodule
