// expected
// $fstrobe writes at the end of the time step (to a file or an MCD; 0 writes nothing);
// $fflush with or without a descriptor.
module top;
  integer fd, mcd, i;
  reg [7:0] v;
  initial begin
    v = 1;
    mcd = 1;
    $fstrobe(mcd, "%0t strobe v=%0d", $time, v);
    v = 2;
    $fflush(mcd);
    $fflush;
    #1 mcd = 0;
    $fstrobe(mcd, "not written");
    fd = $fopen("fstrobe_out.txt", "w");
    $fstrobe(fd, "to file v=%0d", v);
    v = 3;
    #1 $fflush(fd);
    $fclose(fd);
    fd = $fopen("fstrobe_out.txt", "r");
    i = $fgets(v, fd);
    $display("read %0d chars, last %c", i, v);
    $fclose(fd);
  end
endmodule
